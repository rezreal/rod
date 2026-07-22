import { useProgramSwitch } from '../../hooks/useProgramSwitch'

type NavItem = { id: 'hamp' | 'hdsp' | 'hsp' | 'drill' | 'ramp' | 'game' | 'cycle' | 'learn' | 'pulse' | 'impale' | 'plumb' | 'surge' | 'tide' | 'echo' | 'trace' | 'tempo'; label: string; icon: React.ReactNode }

const STANDARD_ITEMS: NavItem[] = [
  {
    id: 'hamp',
    label: 'Oscillate',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M3 12c2-4 4-4 6 0s4 4 6 0 4-4 6 0" />
      </svg>
    ),
  },
  {
    id: 'hdsp',
    label: 'Touchpad',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <rect x="4" y="3" width="16" height="18" rx="3" />
        <circle cx="12" cy="12" r="2.5" fill="currentColor" stroke="none" />
      </svg>
    ),
  },
  {
    id: 'hsp',
    label: 'Script',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <polygon points="5 3 19 12 5 21 5 3" />
      </svg>
    ),
  },
]

const PROGRAM_ITEMS: NavItem[] = [
  {
    id: 'drill',
    label: 'Drill',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <line x1="12" y1="2" x2="12" y2="16" strokeLinecap="round" />
        <polyline points="7 11 12 16 17 11" strokeLinecap="round" strokeLinejoin="round" />
        <line x1="7" y1="20" x2="17" y2="20" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    id: 'ramp',
    label: 'Ramp',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <polyline points="3 18 9 12 13 15 21 6" strokeLinecap="round" strokeLinejoin="round" />
        <polyline points="15 6 21 6 21 12" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    ),
  },
  {
    id: 'game',
    label: 'Games',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M8 21h8M12 17v4M7 4h10v4a5 5 0 0 1-10 0V4z" />
        <path strokeLinecap="round" strokeLinejoin="round" d="M17 5h3v2a3 3 0 0 1-3 3M7 5H4v2a3 3 0 0 0 3 3" />
      </svg>
    ),
  },
  {
    id: 'cycle',
    label: 'Cycle',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M4 12a8 8 0 0 1 13.7-5.6L20 8" />
        <path strokeLinecap="round" strokeLinejoin="round" d="M20 4v4h-4" />
        <path strokeLinecap="round" strokeLinejoin="round" d="M20 12a8 8 0 0 1-13.7 5.6L4 16" />
        <path strokeLinecap="round" strokeLinejoin="round" d="M4 20v-4h4" />
      </svg>
    ),
  },
  {
    id: 'learn',
    label: 'Learn',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M12 4 2 9l10 5 10-5-10-5z" />
        <path strokeLinecap="round" strokeLinejoin="round" d="M6 11v5c0 1 2.7 2.5 6 2.5s6-1.5 6-2.5v-5" />
      </svg>
    ),
  },
  {
    id: 'pulse',
    label: 'Pulse',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M3 12h4l2 6 4-14 2 8h6" />
      </svg>
    ),
  },
  {
    id: 'impale',
    label: 'Impale',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <line x1="12" y1="2" x2="12" y2="18" strokeLinecap="round" />
        <polyline points="8 14 12 18 16 14" strokeLinecap="round" strokeLinejoin="round" />
        <line x1="5" y1="22" x2="19" y2="22" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    id: 'plumb',
    label: 'Plumb',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M12 3v18M8 7l4-4 4 4M8 17l4 4 4-4" />
      </svg>
    ),
  },
  {
    id: 'surge',
    label: 'Surge',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M12 2c1.5 3-1.5 4.5-1.5 7A2.5 2.5 0 0 0 13 11.5c.5 1.5 2 2 2 4a3.5 3.5 0 1 1-7 0c0-3.5 4-5 4-13.5z" />
      </svg>
    ),
  },
  {
    id: 'tide',
    label: 'Tide',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M3 8c2-3 4-3 6 0s4 3 6 0 4-3 6 0M3 16c2-3 4-3 6 0s4 3 6 0 4-3 6 0" />
      </svg>
    ),
  },
  {
    id: 'echo',
    label: 'Echo',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M12 6v12M8 9l-3 3 3 3M16 9l3 3-3 3" />
      </svg>
    ),
  },
  {
    id: 'trace',
    label: 'Trace',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M4 5h16M12 9v9M9 15l3 3 3-3" />
      </svg>
    ),
  },
  {
    id: 'tempo',
    label: 'Tempo',
    icon: (
      <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M9 3h6l3 18H6L9 3zM12 3v8M9.5 13h5" />
      </svg>
    ),
  },
]

interface Props {
  onSettings: () => void
}

export function NavRail({ onSettings }: Props) {
  const { activeProgram, switchProgram } = useProgramSwitch()

  function navButton(item: NavItem) {
    const active = activeProgram === item.id
    return (
      <button
        key={item.id}
        onClick={() => switchProgram(item.id)}
        className={`flex items-center gap-3 w-full px-3 py-3 rounded-xl transition-colors text-sm font-medium
          ${active
            ? 'bg-cyan-500/20 text-cyan-400'
            : 'text-slate-400 hover:text-slate-200 hover:bg-slate-800'
          }`}
      >
        {item.icon}
        <span className="hidden lg:block">{item.label}</span>
      </button>
    )
  }

  return (
    <nav className="hidden md:flex flex-col items-center gap-1 py-4 px-2 bg-slate-900 border-r border-slate-800 w-16 lg:w-48 shrink-0">
      {STANDARD_ITEMS.map(navButton)}

      {/* Programs section */}
      <div className="w-full px-3 pt-3 pb-1">
        <div className="hidden lg:block text-[10px] font-semibold uppercase tracking-widest text-slate-600">
          Programs
        </div>
        <div className="lg:hidden h-px bg-slate-800 w-full" />
      </div>

      {PROGRAM_ITEMS.map(navButton)}

      <div className="flex-1" />

      <button
        onClick={onSettings}
        className="flex items-center gap-3 w-full px-3 py-3 rounded-xl text-slate-400 hover:text-slate-200 hover:bg-slate-800 transition-colors text-sm font-medium"
      >
        <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        <span className="hidden lg:block">Settings</span>
      </button>
    </nav>
  )
}
