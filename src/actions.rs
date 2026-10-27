//! Every command the editor can perform, with the names, icons and shortcuts shown in
//! the menus and tool palette. The menus and palette are both built from this list.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    Open,
    Export,
    ExportSelection,
    FormatPcm16,
    FormatPcm24,
    FormatFloat32,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
    SelectNone,
    Trim,
    Silence,
    FadeIn,
    FadeOut,
    Normalize,
    Gain,
    Reverse,
    ToMono,
    Split,
    PlayPause,
    Stop,
    Loop,
    GoStart,
    GoEnd,
    ZoomIn,
    ZoomOut,
    ZoomSel,
    ZoomFit,
}

use Action::*;

pub const ALL: &[Action] = &[
    Open, Export, ExportSelection, FormatPcm16, FormatPcm24, FormatFloat32, Quit, Undo, Redo, Cut,
    Copy, Paste, Delete, SelectAll, SelectNone, Trim, Silence, FadeIn, FadeOut, Normalize, Gain,
    Reverse, ToMono, Split, PlayPause, Stop, Loop, GoStart, GoEnd, ZoomIn, ZoomOut, ZoomSel, ZoomFit,
];

pub enum Item {
    Do(Action),
    Separator,
    /// A nested submenu.
    Sub(&'static str, &'static [Item]),
}

use Item::{Do, Separator, Sub};

/// Menu bar layout (the macOS app menu with About/Hide/Quit is added separately).
pub const MENUS: &[(&str, &[Item])] = &[
    (
        "File",
        &[
            Do(Open),
            Separator,
            Do(Export),
            Do(ExportSelection),
            Sub("Export Format", &[Do(FormatPcm16), Do(FormatPcm24), Do(FormatFloat32)]),
            Separator,
            Do(Split),
        ],
    ),
    (
        "Edit",
        &[
            Do(Undo),
            Do(Redo),
            Separator,
            Do(Cut),
            Do(Copy),
            Do(Paste),
            Do(Delete),
            Separator,
            Do(SelectAll),
            Do(SelectNone),
            Separator,
            Do(Trim),
            Do(Silence),
        ],
    ),
    (
        "Effects",
        &[Do(FadeIn), Do(FadeOut), Separator, Do(Normalize), Do(Gain), Separator, Do(Reverse), Do(ToMono)],
    ),
    ("Play", &[Do(PlayPause), Do(Stop), Do(Loop), Separator, Do(GoStart), Do(GoEnd)]),
    ("View", &[Do(ZoomIn), Do(ZoomOut), Do(ZoomSel), Do(ZoomFit)]),
];

/// Tool palette layout; `None` is a gap between groups.
pub const PALETTE: &[Option<Action>] = &[
    Some(Open),
    Some(Export),
    None,
    Some(Undo),
    Some(Redo),
    None,
    Some(PlayPause),
    Some(Stop),
    Some(Loop),
    None,
    Some(Trim),
    Some(Delete),
    Some(Silence),
    None,
    Some(Cut),
    Some(Copy),
    Some(Paste),
    None,
    Some(FadeIn),
    Some(FadeOut),
    Some(Normalize),
    Some(Gain),
    None,
    Some(Split),
    None,
    Some(ZoomIn),
    Some(ZoomOut),
    Some(ZoomSel),
    Some(ZoomFit),
];

impl Action {
    pub fn id(self) -> String {
        format!("{self:?}")
    }

    pub fn from_id(id: &str) -> Option<Action> {
        ALL.iter().copied().find(|a| a.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Open => "Open…",
            Export => "Export WAV…",
            ExportSelection => "Export Selection…",
            FormatPcm16 => "16-bit PCM",
            FormatPcm24 => "24-bit PCM",
            FormatFloat32 => "32-bit Float",
            Quit => "Quit Sound Edit",
            Undo => "Undo",
            Redo => "Redo",
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            Delete => "Delete Selection",
            SelectAll => "Select All",
            SelectNone => "Select None",
            Trim => "Trim to Selection",
            Silence => "Silence",
            FadeIn => "Fade In",
            FadeOut => "Fade Out",
            Normalize => "Normalize…",
            Gain => "Gain…",
            Reverse => "Reverse",
            ToMono => "Convert to Mono",
            Split => "Split into Sounds…",
            PlayPause => "Play / Pause",
            Stop => "Stop",
            Loop => "Loop Selection",
            GoStart => "Go to Start",
            GoEnd => "Go to End",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ZoomSel => "Zoom to Selection",
            ZoomFit => "Zoom to Fit",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Open => "📂",
            Export | ExportSelection => "💾",
            Undo => "↶",
            Redo => "↷",
            PlayPause => "▶",
            Stop => "⏹",
            Loop => "🔁",
            Trim => "⛶",
            Delete => "🗑",
            Silence => "🔇",
            Cut => "✂",
            Copy => "🗐",
            Paste => "📋",
            FadeIn => "◢",
            FadeOut => "◣",
            Normalize => "⬍",
            Gain => "🔊",
            Split => "✂…",
            ZoomIn => "🔍+",
            ZoomOut => "🔍−",
            ZoomSel => "⬌",
            ZoomFit => "⛶",
            _ => "",
        }
    }

    /// Native menu accelerator (only ⌘ combinations, so typing in number fields keeps working).
    pub fn accelerator(self) -> Option<&'static str> {
        Some(match self {
            Open => "CmdOrCtrl+O",
            Export => "CmdOrCtrl+S",
            ExportSelection => "CmdOrCtrl+Shift+S",
            Quit => "CmdOrCtrl+Q",
            Undo => "CmdOrCtrl+Z",
            Redo => "CmdOrCtrl+Shift+Z",
            Cut => "CmdOrCtrl+X",
            Copy => "CmdOrCtrl+C",
            Paste => "CmdOrCtrl+V",
            SelectAll => "CmdOrCtrl+A",
            SelectNone => "CmdOrCtrl+D",
            Trim => "CmdOrCtrl+T",
            Normalize => "CmdOrCtrl+Shift+N",
            Gain => "CmdOrCtrl+Shift+G",
            Split => "CmdOrCtrl+Shift+K",
            Loop => "CmdOrCtrl+L",
            ZoomIn => "CmdOrCtrl+Equal",
            ZoomOut => "CmdOrCtrl+Minus",
            ZoomSel => "CmdOrCtrl+E",
            ZoomFit => "CmdOrCtrl+0",
            _ => return None,
        })
    }

    /// Shortcut text for tooltips, including the plain-key ones handled in the window.
    pub fn shortcut(self) -> Option<&'static str> {
        Some(match self {
            Open => "⌘O",
            Export => "⌘S",
            ExportSelection => "⇧⌘S",
            Quit => "⌘Q",
            Undo => "⌘Z",
            Redo => "⇧⌘Z",
            Cut => "⌘X",
            Copy => "⌘C",
            Paste => "⌘V",
            Delete => "⌫",
            SelectAll => "⌘A",
            SelectNone => "⌘D / Esc",
            Trim => "⌘T",
            Normalize => "⇧⌘N",
            Gain => "⇧⌘G",
            Split => "⇧⌘K",
            PlayPause => "Space",
            Stop => "Esc",
            Loop => "⌘L",
            GoStart => "Home",
            GoEnd => "End",
            ZoomIn => "⌘= or =",
            ZoomOut => "⌘- or -",
            ZoomSel => "⌘E or Z",
            ZoomFit => "⌘0 or F",
            _ => return None,
        })
    }

    /// One-line description for the palette tooltip.
    pub fn help(self) -> &'static str {
        match self {
            Trim => "Keep only the selection",
            Delete => "Remove the selection",
            Silence => "Replace the selection with silence",
            Paste => "Insert at the cursor, or replace the selection",
            FadeIn | FadeOut => "Across the selection",
            Normalize | Gain | Reverse => "Applies to the selection, or the whole file if nothing is selected",
            Split => "Save each separate sound as sound01.wav, sound02.wav…",
            PlayPause => "Plays the selection if there is one",
            _ => "",
        }
    }
}
