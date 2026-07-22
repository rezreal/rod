//! Unit conversion and stroke-zone mapping between the Handy RPC world and the
//! actuator's engineering units. See SPEC §7.1.
//!
//! The bridge keeps actuator commands in **engineering units** (mm, mm/s, G);
//! the Modbus driver does the final ×100 conversion to the controller's native
//! 0.01-mm scaling and the hard clamps. This module only handles the
//! *semantic* conversions: percent↔mm, the active stroke zone, and the 8-bit
//! HSP point scale.

/// The active stroke zone `[min, max]` in 0..1 (SPEC slider stroke). All
/// relative positions from clients are remapped into this sub-range so a script
/// authored for the full stroke can be confined to part of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub min: f32,
    pub max: f32,
}

impl Default for Zone {
    fn default() -> Self {
        Zone { min: 0.0, max: 1.0 }
    }
}

impl Zone {
    pub fn new(min: f32, max: f32) -> Self {
        // Keep it well-formed: clamp to [0,1] and ensure min <= max.
        let min = min.clamp(0.0, 1.0);
        let max = max.clamp(0.0, 1.0);
        if min <= max {
            Zone { min, max }
        } else {
            Zone { min: max, max: min }
        }
    }

    /// Map a relative position `p` (0..1) into this zone (0..1 of full stroke).
    pub fn map(&self, p: f32) -> f32 {
        self.min + p.clamp(0.0, 1.0) * (self.max - self.min)
    }
}

/// Conversions parameterised by the actuator stroke length (mm) and the
/// configured max velocity (mm/s) used to scale percentage velocities.
#[derive(Debug, Clone, Copy)]
pub struct Translator {
    pub stroke_mm: f32,
    pub max_velocity_mm_s: f32,
    pub zone: Zone,
}

impl Translator {
    pub fn new(stroke_mm: f32, max_velocity_mm_s: f32) -> Self {
        Translator {
            stroke_mm,
            max_velocity_mm_s,
            zone: Zone::default(),
        }
    }

    /// HDSP `xp` / relative position (0..1) → absolute mm, through the zone.
    pub fn rel_to_mm(&self, p: f32) -> f32 {
        self.zone.map(p) * self.stroke_mm
    }

    /// HSP `Point.x` (0..255) → absolute mm, through the zone.
    pub fn hsp_x_to_mm(&self, x: u8) -> f32 {
        self.rel_to_mm(x as f32 / 255.0)
    }

    /// Clamp an absolute mm target to [0, stroke].
    pub fn clamp_mm(&self, mm: f32) -> f32 {
        mm.clamp(0.0, self.stroke_mm)
    }

    /// HDSP `vp` / HAMP velocity (percent 0..1) → mm/s.
    pub fn vel_pct_to_mm_s(&self, vp: f32) -> f32 {
        vp.clamp(0.0, 1.0) * self.max_velocity_mm_s
    }

    /// Velocity (mm/s) needed to travel `distance_mm` in `t` milliseconds.
    /// Used by the `…T` (duration) HDSP variants and HSP point playback.
    pub fn duration_to_vel(distance_mm: f32, t_ms: u32) -> f32 {
        let t = t_ms.max(1) as f32;
        distance_mm.abs() / (t / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_maps_into_subrange() {
        let z = Zone::new(0.2, 0.8);
        assert!((z.map(0.0) - 0.2).abs() < 1e-6);
        assert!((z.map(1.0) - 0.8).abs() < 1e-6);
        assert!((z.map(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn zone_normalises_inverted_and_out_of_range_bounds() {
        let z = Zone::new(0.9, 0.1);
        assert_eq!((z.min, z.max), (0.1, 0.9));
        let z2 = Zone::new(-1.0, 2.0);
        assert_eq!((z2.min, z2.max), (0.0, 1.0));
    }

    #[test]
    fn rel_and_hsp_conversions_respect_zone_and_stroke() {
        let mut t = Translator::new(300.0, 400.0);
        // Full stroke, no zone.
        assert_eq!(t.rel_to_mm(1.0), 300.0);
        assert_eq!(t.rel_to_mm(0.5), 150.0);
        assert_eq!(t.hsp_x_to_mm(255), 300.0);
        assert!((t.hsp_x_to_mm(128) - (128.0 / 255.0 * 300.0)).abs() < 1e-3);
        // With a zone of [0.5, 1.0]: 0.0 -> 150mm, 1.0 -> 300mm.
        t.zone = Zone::new(0.5, 1.0);
        assert_eq!(t.rel_to_mm(0.0), 150.0);
        assert_eq!(t.rel_to_mm(1.0), 300.0);
    }

    #[test]
    fn velocity_conversions() {
        let t = Translator::new(300.0, 400.0);
        assert_eq!(t.vel_pct_to_mm_s(1.0), 400.0);
        assert_eq!(t.vel_pct_to_mm_s(0.25), 100.0);
        // 60 mm in 200 ms = 300 mm/s.
        assert!((Translator::duration_to_vel(60.0, 200) - 300.0).abs() < 1e-3);
        // Guard against divide-by-zero.
        assert!(Translator::duration_to_vel(10.0, 0).is_finite());
    }
}
