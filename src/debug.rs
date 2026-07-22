//! Local raw-Modbus debug console.
//!
//! A line-based TCP server (loopback only) that turns ASCII commands into
//! [`BridgeCommand`]s on the driver's bridge channel, so the controller can be
//! poked directly over SSH while investigating the Modbus protocol — no BLE
//! central, no rebuild per experiment:
//!
//! ```text
//! nc 127.0.0.1 7878
//! rreg 0x9000 10                 # read 10 status regs  -> ok <w0> <w1> …
//! wreg 0x9900 0 27000 0 10 …     # write the move block (FC 0x10)
//! wcoil 0x0403 1                 # servo on (FC 0x05)
//! testmove 27000 10000 30 0x40   # atomic: write move, settle, read ALMC
//! status                         # decoded status block
//! reset-alarm | calibrate
//! ```
//!
//! Numbers are decimal, or hex with a `0x` prefix. **Unauthenticated raw bus
//! access** — keep it bound to loopback and off by default.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::modbus::protocol;
use crate::state::BridgeCommand;

/// Bind the debug console and serve connections until the process exits.
pub async fn run(listen: &str, bridge_tx: mpsc::Sender<BridgeCommand>) {
    let listener = match TcpListener::bind(listen).await {
        Ok(l) => l,
        Err(e) => {
            warn!(listen, error = %e, "debug console: bind failed; disabled");
            return;
        }
    };
    info!(listen, "debug console listening (raw Modbus over TCP)");
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let tx = bridge_tx.clone();
                tokio::spawn(async move {
                    info!(%peer, "debug console: client connected");
                    if let Err(e) = serve_conn(stream, tx).await {
                        warn!(%peer, error = %e, "debug console: connection ended");
                    }
                });
            }
            Err(e) => warn!(error = %e, "debug console: accept failed"),
        }
    }
}

/// Read lines from one connection, dispatch each, and write the reply line.
async fn serve_conn(
    stream: tokio::net::TcpStream,
    bridge_tx: mpsc::Sender<BridgeCommand>,
) -> std::io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut lines = BufReader::new(rd).lines();
    wr.write_all(b"rod debug console. commands: rreg wreg wcoil testmove status reset-alarm calibrate peck-probe help\n").await?;
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "quit" || line == "exit" {
            break;
        }
        let resp = dispatch_line(line, &bridge_tx).await;
        wr.write_all(resp.as_bytes()).await?;
        wr.write_all(b"\n").await?;
    }
    Ok(())
}

/// Parse a numeric token: hex with a `0x` prefix, otherwise decimal.
fn parse_num(tok: &str) -> Result<u32, String> {
    let t = tok.trim();
    let r = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        t.parse::<u32>()
    };
    r.map_err(|_| format!("bad number {tok:?}"))
}

/// Send a `BridgeCommand` and await its reply, mapping channel errors to text.
async fn round_trip<T>(
    bridge_tx: &mpsc::Sender<BridgeCommand>,
    cmd: BridgeCommand,
    rx: oneshot::Receiver<Result<T, String>>,
) -> Result<T, String> {
    if bridge_tx.send(cmd).await.is_err() {
        return Err("bridge offline".into());
    }
    match rx.await {
        Ok(r) => r,
        Err(_) => Err("no reply".into()),
    }
}

/// Map one ASCII command line to a [`BridgeCommand`] and format the reply.
pub async fn dispatch_line(line: &str, bridge_tx: &mpsc::Sender<BridgeCommand>) -> String {
    let mut tok = line.split_whitespace();
    let verb = tok.next().unwrap_or("");
    match verb {
        "help" => "rreg <addr> <count> | wreg <addr> <w0..> | wcoil <addr> <0|1> | \
                   testmove <pos_001> <vel_001> <accel_001> <ctlf> | status | reset-alarm | calibrate | peck-probe"
            .to_string(),

        "rreg" => {
            let (addr, count) = match (tok.next().map(parse_num), tok.next().map(parse_num)) {
                (Some(Ok(a)), Some(Ok(c))) if a <= 0xffff && (1..=125).contains(&c) => {
                    (a as u16, c as u16)
                }
                (Some(Ok(_)), Some(Ok(_))) => return "err addr<=0xffff, 1<=count<=125".into(),
                (Some(Err(e)), _) | (_, Some(Err(e))) => return format!("err {e}"),
                _ => return "err usage: rreg <addr> <count>".into(),
            };
            let (reply, rx) = oneshot::channel();
            match round_trip(
                bridge_tx,
                BridgeCommand::RawReadRegs { addr, count, reply },
                rx,
            )
            .await
            {
                Ok(words) => format!(
                    "ok {}",
                    words
                        .iter()
                        .map(|w| format!("{w:04x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                Err(e) => format!("err {e}"),
            }
        }

        "wreg" => {
            let addr = match tok.next().map(parse_num) {
                Some(Ok(a)) if a <= 0xffff => a as u16,
                Some(Ok(_)) => return "err addr<=0xffff".into(),
                Some(Err(e)) => return format!("err {e}"),
                None => return "err usage: wreg <addr> <w0> [w1 …]".into(),
            };
            let mut data = Vec::new();
            for t in tok {
                match parse_num(t) {
                    Ok(w) if w <= 0xffff => data.push(w as u16),
                    Ok(_) => return format!("err word {t:?} > 0xffff"),
                    Err(e) => return format!("err {e}"),
                }
            }
            if data.is_empty() {
                return "err usage: wreg <addr> <w0> [w1 …]".into();
            }
            let (reply, rx) = oneshot::channel();
            match round_trip(
                bridge_tx,
                BridgeCommand::RawWriteRegs { addr, data, reply },
                rx,
            )
            .await
            {
                Ok(()) => "ok wreg".into(),
                Err(e) => format!("err {e}"),
            }
        }

        "wcoil" => {
            let (addr, on) = match (tok.next().map(parse_num), tok.next()) {
                (Some(Ok(a)), Some(v)) if a <= 0xffff => (
                    a as u16,
                    v == "1" || v.eq_ignore_ascii_case("on") || v.eq_ignore_ascii_case("true"),
                ),
                (Some(Ok(_)), Some(_)) => return "err addr<=0xffff".into(),
                (Some(Err(e)), _) => return format!("err {e}"),
                _ => return "err usage: wcoil <addr> <0|1>".into(),
            };
            let (reply, rx) = oneshot::channel();
            match round_trip(
                bridge_tx,
                BridgeCommand::RawWriteCoil { addr, on, reply },
                rx,
            )
            .await
            {
                Ok(()) => format!("ok wcoil {addr:#06x}={}", on as u8),
                Err(e) => format!("err {e}"),
            }
        }

        // Atomic CTLF probe: write the move block, settle, read ALMC back —
        // immune to the status-poll / auto-reset clearing the alarm first.
        "testmove" => {
            let nums: Vec<u32> = match tok.map(parse_num).collect::<Result<Vec<_>, _>>() {
                Ok(v) => v,
                Err(e) => return format!("err {e}"),
            };
            if nums.len() != 4 {
                return "err usage: testmove <pos_001> <vel_001> <accel_001> <ctlf>".into();
            }
            let (pos, vel, accel, ctlf) = (nums[0], nums[1], nums[2], nums[3]);
            let regs: [u16; 9] = [
                (pos >> 16) as u16,
                pos as u16,
                0,
                10, // 0.10mm positioning band
                (vel >> 16) as u16,
                vel as u16,
                accel as u16,
                0, // push current
                ctlf as u16,
            ];
            let (reply, rx) = oneshot::channel();
            match round_trip(
                bridge_tx,
                BridgeCommand::RawTestMove {
                    regs,
                    settle_ms: 150,
                    reply,
                },
                rx,
            )
            .await
            {
                Ok(s) if s.len() >= 3 => {
                    let pnow = ((s[0] as i32) << 16) | s[1] as i32;
                    format!(
                        "ok almc={:04x} pnow={} (={:.2}mm)",
                        s[2],
                        pnow,
                        pnow as f32 / 100.0
                    )
                }
                Ok(_) => "err short status".into(),
                Err(e) => format!("err {e}"),
            }
        }

        "status" => {
            let (reply, rx) = oneshot::channel();
            let cmd = BridgeCommand::RawReadRegs {
                addr: protocol::REG_STATUS_BLOCK,
                count: protocol::STATUS_BLOCK_LEN,
                reply,
            };
            match round_trip(bridge_tx, cmd, rx).await {
                Ok(s) if s.len() >= protocol::STATUS_BLOCK_LEN as usize => {
                    let pnow = ((s[0] as i32) << 16) | s[1] as i32;
                    format!(
                        "ok pnow={} (={:.2}mm) almc={:04x} dipm={:04x} dipo={:04x} \
                         dss1={:04x} dss2={:04x} dsse={:04x} stat={:04x}{:04x}",
                        pnow, pnow as f32 / 100.0, s[2], s[3], s[4], s[5], s[6], s[7], s[8], s[9]
                    )
                }
                Ok(_) => "err short status".into(),
                Err(e) => format!("err {e}"),
            }
        }

        "reset-alarm" => {
            let (reply, rx) = oneshot::channel();
            match round_trip(
                bridge_tx,
                BridgeCommand::ResetAlarm { reply },
                rx,
            )
            .await
            {
                Ok(()) => "ok reset-alarm".into(),
                Err(e) => format!("err {e}"),
            }
        }

        "calibrate" => {
            let (reply, rx) = oneshot::channel();
            match round_trip(
                bridge_tx,
                BridgeCommand::Calibrate { reply },
                rx,
            )
            .await
            {
                Ok(pos) => format!("ok contact {pos:.2}"),
                Err(e) => format!("err {e}"),
            }
        }

        "peck-probe" => {
            let (reply, rx) = oneshot::channel();
            match round_trip(bridge_tx, BridgeCommand::PeckProbe { reply }, rx).await {
                Ok(pos) => format!("ok work_origin={pos:.2}mm"),
                Err(e) => format!("err {e}"),
            }
        }

        other => format!("err unknown command {other:?} (try: help)"),
    }
}
