//! What a right-click on an app offers: the same rows, words, pictograms and order in the dock, on a window's tile, and in the
//! launcher. Each place turns the rows into its own menu (the dock's widget menu, the launcher's own) and does them its own way.

/// What is known of the app where it was right-clicked.
#[derive(Default)]
pub struct Facts {
    /// the place's press does not open it (the dock: it shows its windows), so the menu does
    pub open_row: bool,
    /// how many of its windows are open (none: not running)
    pub windows: usize,
    /// the menu is a single window's (a tile, a window found in the launcher): Close closes that one
    pub one_window: bool,
    pub in_dock: bool,
    pub in_apps: bool,
    /// on disk (a program, a shortcut, a folder): where it is can be opened
    pub location: bool,
    /// a program (it runs as administrator)
    pub program: bool,
    /// a pin of the place's own (its name and picture can be changed); whether its picture was changed
    pub own_pin: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Open,
    DockPin,
    AppsPin,
    Location,
    Admin,
    Rename,
    ChangeIcon,
    ResetIcon,
    Close,
    EndTask,
}

pub struct Row {
    pub icon: &'static str,
    pub label: &'static str,
    pub act: Act,
    /// it cannot be undone: red, held and asked first
    pub danger: bool,
}

/// What End Task asks before it ends anything.
pub const END_TASK_WARNING: &str = "What it has not saved will be lost.";

pub fn rows(f: &Facts) -> Vec<Row> {
    let row = |icon, label, act| Row { icon, label, act, danger: false };
    let running = f.windows > 0;
    let mut out = Vec::new();
    if f.open_row {
        out.push(row("window-new", if running { "New Window" } else { "Open" }, Act::Open));
    }
    out.push(if f.in_dock { row("unpin", "Unpin from Dock", Act::DockPin) } else { row("pin", "Pin to Dock", Act::DockPin) });
    out.push(if f.in_apps { row("grid", "Unpin from Apps", Act::AppsPin) } else { row("grid", "Pin to Apps", Act::AppsPin) });
    if f.location || f.program {
        out.push(row("folder", "Open File Location", Act::Location));
    }
    if f.program {
        out.push(row("shield", "Run as Administrator", Act::Admin));
    }
    if let Some(changed) = f.own_pin {
        out.push(row("pen", "Rename", Act::Rename));
        out.push(row("image", "Change Icon", Act::ChangeIcon));
        if changed {
            out.push(row("reset", "Reset Icon", Act::ResetIcon));
        }
    }
    if running {
        out.push(row("x", if f.windows > 1 && !f.one_window { "Close All" } else { "Close" }, Act::Close));
        out.push(Row { icon: "power", label: "End Task", act: Act::EndTask, danger: true });
    }
    out
}
