use eframe::egui;
use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, Event, FontId, Key, Pos2, Rect, Sense, Stroke,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Storage key under which the in-progress puzzle is persisted.
const STATE_KEY: &str = "sudokah_state";
/// Storage key for the best solve time (seconds) per difficulty.
const BEST_TIMES_KEY: &str = "sudokah_best_times";

/// `MM:SS`, or `H:MM:SS` once the solve passes an hour.
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Run the app with the given native options (shared by desktop and Android).
fn run(options: eframe::NativeOptions) -> eframe::Result<()> {
    eframe::run_native(
        "Sudokah",
        options,
        Box::new(|cc| Ok(Box::new(SudokahApp::new(cc)))),
    )
}

/// Desktop entry point, called from `src/main.rs`.
pub fn run_desktop() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 860.0])
            .with_min_inner_size([560.0, 680.0])
            .with_title("Sudokah"),
        ..Default::default()
    };
    run(options)
}

/// Android entry point. `android-activity` (via winit) calls this symbol after
/// the native activity starts; we hand it the `AndroidApp` eframe needs.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::activity::WindowManagerFlags;

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    // Keep the screen on while playing.
    app.set_window_flags(WindowManagerFlags::KEEP_SCREEN_ON, WindowManagerFlags::empty());

    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    if let Err(e) = run(options) {
        log::error!("sudokah exited with error: {e}");
    }
}

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Cell {
    given: bool,
    value: Option<u8>,
    corner: [bool; 9],
    center: [bool; 9],
    color: Option<usize>,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            given: false,
            value: None,
            corner: [false; 9],
            center: [false; 9],
            color: None,
        }
    }
}

type Grid = [[Cell; 9]; 9];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    Normal,
    Corner,
    Center,
    Color,
}

// Pastel highlight palette (SudokuPad-ish).
const COLORS: [Color32; 9] = [
    Color32::from_rgb(207, 207, 207), // grey
    Color32::from_rgb(252, 175, 175), // red
    Color32::from_rgb(255, 213, 153), // orange
    Color32::from_rgb(255, 247, 153), // yellow
    Color32::from_rgb(190, 232, 167), // green
    Color32::from_rgb(167, 222, 232), // cyan
    Color32::from_rgb(178, 196, 247), // blue
    Color32::from_rgb(214, 184, 247), // purple
    Color32::from_rgb(247, 184, 224), // pink
];

// Ink colours, shared by the board and the keypad so a digit in the pad matches
// what it draws on the grid in the current mode.
const USER_COL: Color32 = Color32::from_rgb(28, 88, 214); // placed digit: blue
const CENTER_COL: Color32 = Color32::from_rgb(0, 0, 128); // center marks: navy
const CORNER_COL: Color32 = Color32::from_rgb(120, 72, 0); // corner marks: brown

impl Mode {
    /// The colour digits take in this mode (Color mode has no digit ink).
    fn ink(self) -> Color32 {
        match self {
            Mode::Normal => USER_COL,
            Mode::Center => CENTER_COL,
            Mode::Corner => CORNER_COL,
            Mode::Color => USER_COL,
        }
    }
}

/// A destructive action that, when the board has unsaved progress, waits for an
/// "are you sure?" confirmation before running.
#[derive(Clone)]
enum PendingAction {
    NewPuzzle(String),
    Solve,
    ClearAll,
}

struct SudokahApp {
    grid: Grid,
    /// The board as it was when last loaded/cleared; used to tell whether the
    /// user has actually changed anything since.
    baseline: Grid,
    selection: Vec<(usize, usize)>,
    mode: Mode,
    set_givens: bool,
    show_auto_candidates: bool,
    show_errors: bool,
    /// The puzzle's solution (from the givens), computed when a puzzle loads, so
    /// "Show errors" can flag digits that don't match it.
    solution: Option<[[u8; 9]; 9]>,
    undo: Vec<Grid>,
    redo: Vec<Grid>,
    load_text: String,
    show_load_dialog: bool,
    /// Why the last manual load failed (empty when none); shown in the dialog.
    load_error: String,
    /// A destructive action awaiting confirmation (see [`PendingAction`]).
    pending: Option<PendingAction>,
    /// Difficulty of the active puzzle ("easy".."expert"), or `None` for a
    /// hand-loaded puzzle. Selects the best-time bucket on completion.
    difficulty: Option<String>,
    /// When the running timer segment began; `None` when the clock is stopped
    /// (no active puzzle, or already solved).
    timer_start: Option<Instant>,
    /// Solve time accumulated before the current running segment (and the frozen
    /// total once solved).
    timer_elapsed: Duration,
    /// True once the active puzzle is finished and the clock is frozen.
    solved: bool,
    /// Set when the solver filled this puzzle: it then earns no best time, no
    /// matter how the board reaches completion.
    solver_used: bool,
    /// Set when the just-finished solve beat the stored best for its difficulty.
    new_best: bool,
    /// Best solve time in whole seconds, keyed by difficulty.
    best_times: HashMap<String, u64>,
    /// Whether the best-times pop-up is open.
    show_best_times: bool,
    /// The title last pushed to the window, so we only resend on change.
    last_title: String,
}

/// The slice of app state persisted between sessions (an in-progress puzzle).
#[derive(Serialize, Deserialize)]
struct SaveState {
    grid: Grid,
    #[serde(default = "empty_grid")]
    baseline: Grid,
    set_givens: bool,
    mode: Mode,
    #[serde(default)]
    show_errors: bool,
    /// Difficulty of the in-progress puzzle, if it came from a generator.
    #[serde(default)]
    difficulty: Option<String>,
    /// Solve time banked so far, in milliseconds.
    #[serde(default)]
    elapsed_ms: u64,
}

fn empty_grid() -> Grid {
    [[Cell::default(); 9]; 9]
}

impl Default for SudokahApp {
    fn default() -> Self {
        SudokahApp {
            grid: [[Cell::default(); 9]; 9],
            baseline: [[Cell::default(); 9]; 9],
            selection: Vec::new(),
            mode: Mode::Normal,
            set_givens: true,
            show_auto_candidates: false,
            show_errors: false,
            solution: None,
            undo: Vec::new(),
            redo: Vec::new(),
            load_text: String::new(),
            show_load_dialog: false,
            load_error: String::new(),
            pending: None,
            difficulty: None,
            timer_start: None,
            timer_elapsed: Duration::ZERO,
            solved: false,
            solver_used: false,
            new_best: false,
            best_times: HashMap::new(),
            show_best_times: false,
            last_title: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Edit operations
// ---------------------------------------------------------------------------

impl SudokahApp {
    /// Build the app, restoring an in-progress puzzle from storage if present.
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Default to light mode regardless of the system theme.
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let mut app = SudokahApp::default();
        if let Some(storage) = cc.storage {
            if let Some(json) = storage.get_string(STATE_KEY) {
                if let Ok(state) = serde_json::from_str::<SaveState>(&json) {
                    app.grid = state.grid;
                    app.baseline = state.baseline;
                    app.set_givens = state.set_givens;
                    app.mode = state.mode;
                    app.show_errors = state.show_errors;
                    if app.show_errors {
                        app.compute_solution();
                    }
                    // Resume the clock from where the saved puzzle left off.
                    if !app.is_empty() && !app.is_completed() {
                        app.difficulty = state.difficulty;
                        app.timer_elapsed = Duration::from_millis(state.elapsed_ms);
                        app.timer_start = Some(Instant::now());
                    }
                }
            }
            if let Some(json) = storage.get_string(BEST_TIMES_KEY) {
                if let Ok(best) = serde_json::from_str::<HashMap<String, u64>>(&json) {
                    app.best_times = best;
                }
            }
        }
        app
    }

    /// True once every cell holds a final, conflict-free digit.
    fn is_completed(&self) -> bool {
        for r in 0..9 {
            for c in 0..9 {
                match self.grid[r][c].value {
                    Some(d) if self.is_legal(r, c, d) => {}
                    _ => return false,
                }
            }
        }
        true
    }

    /// True if the board is untouched (nothing worth persisting).
    fn is_empty(&self) -> bool {
        self.grid
            .iter()
            .flatten()
            .all(|cell| *cell == Cell::default())
    }

    /// True when the user has unfinished work a destructive action would lose.
    fn needs_confirm(&self) -> bool {
        self.grid != self.baseline && !self.is_completed()
    }

    fn push_undo(&mut self) {
        self.undo.push(self.grid);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self) {
        if let Some(g) = self.undo.pop() {
            self.redo.push(self.grid);
            self.grid = g;
        }
    }

    fn redo(&mut self) {
        if let Some(g) = self.redo.pop() {
            self.undo.push(self.grid);
            self.grid = g;
        }
    }

    /// Cells in the current selection that are editable in the active context.
    fn editable_selection(&self) -> Vec<(usize, usize)> {
        self.selection
            .iter()
            .copied()
            .filter(|&(r, c)| self.set_givens || !self.grid[r][c].given)
            .collect()
    }

    fn apply_digit(&mut self, d: u8) {
        let cells = self.editable_selection();
        if cells.is_empty() {
            return;
        }
        self.push_undo();
        match self.mode {
            Mode::Normal => {
                for &(r, c) in &cells {
                    if self.grid[r][c].value == Some(d) {
                        self.grid[r][c].value = None;
                        if self.set_givens {
                            self.grid[r][c].given = false;
                        }
                    } else {
                        let cell = &mut self.grid[r][c];
                        cell.value = Some(d);
                        cell.given = self.set_givens;
                        cell.corner = [false; 9];
                        cell.center = [false; 9];
                        // Placing a digit invalidates that candidate in its peers.
                        self.remove_peer_marks(r, c, d);
                    }
                }
            }
            Mode::Corner => self.toggle_marks(&cells, d, false),
            Mode::Center => self.toggle_marks(&cells, d, true),
            Mode::Color => {
                let idx = (d - 1) as usize;
                let all = cells.iter().all(|&(r, c)| self.grid[r][c].color == Some(idx));
                for &(r, c) in &cells {
                    self.grid[r][c].color = if all { None } else { Some(idx) };
                }
            }
        }
    }

    /// Clear digit `d` from the center candidate marks of every cell that shares
    /// a row, column, or box with `(r, c)`. Corner marks are left untouched.
    fn remove_peer_marks(&mut self, r: usize, c: usize, d: u8) {
        let i = (d - 1) as usize;
        let clear = |cell: &mut Cell| {
            cell.center[i] = false;
        };
        for k in 0..9 {
            clear(&mut self.grid[r][k]);
            clear(&mut self.grid[k][c]);
        }
        let (br, bc) = (r / 3 * 3, c / 3 * 3);
        for i2 in 0..3 {
            for j2 in 0..3 {
                clear(&mut self.grid[br + i2][bc + j2]);
            }
        }
    }

    fn toggle_marks(&mut self, cells: &[(usize, usize)], d: u8, center: bool) {
        let i = (d - 1) as usize;
        // Only mark cells without a final digit.
        let targets: Vec<(usize, usize)> = cells
            .iter()
            .copied()
            .filter(|&(r, c)| self.grid[r][c].value.is_none())
            .collect();
        if targets.is_empty() {
            return;
        }
        let all_set = targets.iter().all(|&(r, c)| {
            if center {
                self.grid[r][c].center[i]
            } else {
                self.grid[r][c].corner[i]
            }
        });
        for &(r, c) in &targets {
            let cell = &mut self.grid[r][c];
            if center {
                cell.center[i] = !all_set;
            } else {
                cell.corner[i] = !all_set;
            }
        }
    }

    /// Layered delete: value first, then pencil marks, then color.
    fn clear_selected(&mut self) {
        let cells = self.editable_selection();
        if cells.is_empty() {
            return;
        }
        self.push_undo();
        for &(r, c) in &cells {
            let cell = &mut self.grid[r][c];
            if cell.value.is_some() {
                cell.value = None;
                if self.set_givens {
                    cell.given = false;
                }
            } else if cell.center.iter().any(|&x| x) || cell.corner.iter().any(|&x| x) {
                cell.center = [false; 9];
                cell.corner = [false; 9];
            } else {
                cell.color = None;
            }
        }
    }

    fn clear_all(&mut self) {
        self.push_undo();
        self.grid = [[Cell::default(); 9]; 9];
        self.baseline = self.grid;
        self.stop_timer();
    }

    /// Total solve time so far: banked time plus the running segment.
    fn elapsed(&self) -> Duration {
        self.timer_elapsed
            + self
                .timer_start
                .map_or(Duration::ZERO, |start| start.elapsed())
    }

    /// Begin timing a fresh puzzle of the given difficulty (`None` = loaded).
    fn start_timer(&mut self, difficulty: Option<String>) {
        self.difficulty = difficulty;
        self.timer_elapsed = Duration::ZERO;
        self.timer_start = Some(Instant::now());
        self.solved = false;
        self.solver_used = false;
        self.new_best = false;
    }

    /// Stop and reset the clock (no active puzzle).
    fn stop_timer(&mut self) {
        self.difficulty = None;
        self.timer_start = None;
        self.timer_elapsed = Duration::ZERO;
        self.solved = false;
        self.solver_used = false;
        self.new_best = false;
    }

    /// Freeze the clock without recording a best time (used when the solver
    /// fills the board, which shouldn't count as the player's own time).
    fn freeze_timer(&mut self) {
        self.timer_elapsed = self.elapsed();
        self.timer_start = None;
        self.solved = true;
        self.solver_used = true;
        self.new_best = false;
    }

    /// Once per frame: if the player has just completed the puzzle, freeze the
    /// clock and update the best time. Keeps repainting while the clock runs so
    /// the title-bar timer ticks.
    fn update_timer(&mut self, ctx: &egui::Context) {
        if self.solved || self.timer_start.is_none() {
            return;
        }
        if self.is_completed() {
            self.timer_elapsed = self.elapsed();
            self.timer_start = None;
            self.solved = true;
            self.record_solve();
        } else {
            // Tick the displayed time roughly four times a second.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Bank the finished solve time against the difficulty's best, flagging
    /// [`Self::new_best`] when an existing record was beaten. A first-ever solve
    /// is stored but not celebrated. No-op for hand-loaded puzzles.
    fn record_solve(&mut self) {
        // A board the solver finished never earns a best time.
        if self.solver_used {
            return;
        }
        let Some(diff) = self.difficulty.clone() else {
            return;
        };
        let secs = self.timer_elapsed.as_secs();
        let prev = self.best_times.get(&diff).copied();
        if prev.is_none_or(|b| secs < b) {
            self.best_times.insert(diff, secs);
            self.new_best = prev.is_some();
        }
    }

    /// Reflect the solve clock in the window title, e.g. "Sudokah — 02:35".
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = if self.solved {
            format!("Sudokah — Solved {}", format_duration(self.timer_elapsed))
        } else if self.timer_start.is_some() {
            format!("Sudokah — {}", format_duration(self.elapsed()))
        } else {
            "Sudokah".to_owned()
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn clear_pencil_marks(&mut self) {
        self.push_undo();
        for row in self.grid.iter_mut() {
            for cell in row.iter_mut() {
                cell.center = [false; 9];
                cell.corner = [false; 9];
            }
        }
    }

    /// The legal digits for an empty cell, as a center-mark bitmap.
    fn cell_candidates(&self, r: usize, c: usize) -> [bool; 9] {
        let mut center = [false; 9];
        for d in 1..=9u8 {
            if self.is_legal(r, c, d) {
                center[(d - 1) as usize] = true;
            }
        }
        center
    }

    /// How many of each digit (1-9) are placed as final values on the board.
    fn digit_counts(&self) -> [u8; 9] {
        let mut counts = [0u8; 9];
        for row in &self.grid {
            for cell in row {
                if let Some(d) = cell.value {
                    counts[(d - 1) as usize] += 1;
                }
            }
        }
        counts
    }

    fn is_legal(&self, r: usize, c: usize, d: u8) -> bool {
        for k in 0..9 {
            if k != c && self.grid[r][k].value == Some(d) {
                return false;
            }
            if k != r && self.grid[k][c].value == Some(d) {
                return false;
            }
        }
        let (br, bc) = (r / 3 * 3, c / 3 * 3);
        for i in 0..3 {
            for j in 0..3 {
                let (rr, cc) = (br + i, bc + j);
                if (rr != r || cc != c) && self.grid[rr][cc].value == Some(d) {
                    return false;
                }
            }
        }
        true
    }

    fn solve(&mut self) {
        let mut board = [[0u8; 9]; 9];
        for r in 0..9 {
            for c in 0..9 {
                board[r][c] = self.grid[r][c].value.unwrap_or(0);
            }
        }
        if !board_consistent(&board) {
            return;
        }
        if backtrack(&mut board) {
            self.push_undo();
            for r in 0..9 {
                for c in 0..9 {
                    if !self.grid[r][c].given {
                        self.grid[r][c].value = Some(board[r][c]);
                        self.grid[r][c].center = [false; 9];
                        self.grid[r][c].corner = [false; 9];
                    }
                }
            }
            // Solving for the player freezes the clock but earns no best time.
            self.freeze_timer();
        }
    }

    /// Parse and load the puzzle in `load_text`. Returns `true` on success;
    /// on failure sets `load_error` describing why (shown in the dialog).
    fn load_from_text(&mut self) -> bool {
        let digits: Vec<u8> = self
            .load_text
            .chars()
            .filter_map(|ch| match ch {
                '1'..='9' => Some(ch as u8 - b'0'),
                '0' | '.' | '-' => Some(0),
                _ => None,
            })
            .collect();
        if digits.len() != 81 {
            self.load_error = format!("Need 81 cells, got {}.", digits.len());
            return false;
        }
        let mut board = [[0u8; 9]; 9];
        for (idx, &d) in digits.iter().enumerate() {
            board[idx / 9][idx % 9] = d;
        }
        if !board_consistent(&board) {
            self.load_error = "Puzzle has a contradiction.".to_owned();
            return false;
        }
        match count_solutions(&mut board.clone(), 2) {
            0 => {
                self.load_error = "Puzzle has no solution.".to_owned();
                false
            }
            1 => {
                self.set_board_givens(&board);
                self.start_timer(None);
                self.load_error.clear();
                true
            }
            _ => {
                self.load_error = "Puzzle has multiple solutions.".to_owned();
                false
            }
        }
    }

    /// Replace the board with a fresh puzzle, locking non-zero cells as givens.
    fn set_board_givens(&mut self, board: &[[u8; 9]; 9]) {
        self.push_undo();
        self.grid = [[Cell::default(); 9]; 9];
        for r in 0..9 {
            for c in 0..9 {
                if board[r][c] != 0 {
                    let cell = &mut self.grid[r][c];
                    cell.value = Some(board[r][c]);
                    cell.given = true;
                }
            }
        }
        self.selection.clear();
        self.set_givens = false;
        self.baseline = self.grid;
        self.compute_solution();
    }

    /// Solve the puzzle from the current givens and cache it for "Show errors".
    fn compute_solution(&mut self) {
        let mut board = [[0u8; 9]; 9];
        for r in 0..9 {
            for c in 0..9 {
                if self.grid[r][c].given {
                    board[r][c] = self.grid[r][c].value.unwrap_or(0);
                }
            }
        }
        self.solution = if board_consistent(&board) && backtrack(&mut board) {
            Some(board)
        } else {
            None
        };
    }

    /// Generate a fresh, uniquely-solvable puzzle at the given difficulty.
    fn new_puzzle(&mut self, difficulty: &str) {
        let mut rng = Rng::new();
        let target_givens = match difficulty {
            "easy" => 40,
            "medium" => 32,
            "hard" => 26,
            _ => 23, // "expert": fewest clues
        };
        let board = generate_puzzle(target_givens, &mut rng);
        self.set_board_givens(&board);
        self.start_timer(Some(difficulty.to_owned()));
    }
}

// ---------------------------------------------------------------------------
// Solver
// ---------------------------------------------------------------------------

fn board_consistent(b: &[[u8; 9]; 9]) -> bool {
    for r in 0..9 {
        for c in 0..9 {
            let d = b[r][c];
            if d != 0 && !legal(b, r, c, d) {
                return false;
            }
        }
    }
    true
}

fn legal(b: &[[u8; 9]; 9], r: usize, c: usize, d: u8) -> bool {
    for k in 0..9 {
        if k != c && b[r][k] == d {
            return false;
        }
        if k != r && b[k][c] == d {
            return false;
        }
    }
    let (br, bc) = (r / 3 * 3, c / 3 * 3);
    for i in 0..3 {
        for j in 0..3 {
            if (br + i != r || bc + j != c) && b[br + i][bc + j] == d {
                return false;
            }
        }
    }
    true
}

fn backtrack(b: &mut [[u8; 9]; 9]) -> bool {
    // Find the empty cell with the fewest candidates (MRV heuristic).
    let mut best: Option<(usize, usize, Vec<u8>)> = None;
    for r in 0..9 {
        for c in 0..9 {
            if b[r][c] == 0 {
                let cands: Vec<u8> = (1..=9).filter(|&d| legal(b, r, c, d)).collect();
                if cands.is_empty() {
                    return false;
                }
                if best.as_ref().map_or(true, |x| cands.len() < x.2.len()) {
                    let stop = cands.len() == 1;
                    best = Some((r, c, cands));
                    if stop {
                        break;
                    }
                }
            }
        }
    }
    match best {
        None => true, // no empties => solved
        Some((r, c, cands)) => {
            for d in cands {
                b[r][c] = d;
                if backtrack(b) {
                    return true;
                }
                b[r][c] = 0;
            }
            false
        }
    }
}

/// Count the puzzle's solutions, stopping once `limit` are found. A proper
/// puzzle returns exactly 1; `>= 2` means it's ambiguous. Uses the MRV
/// heuristic so it stays fast even on sparse, hard puzzles.
fn count_solutions(b: &mut [[u8; 9]; 9], limit: usize) -> usize {
    let mut best: Option<(usize, usize, Vec<u8>)> = None;
    'find: for r in 0..9 {
        for c in 0..9 {
            if b[r][c] == 0 {
                let cands: Vec<u8> = (1..=9).filter(|&d| legal(b, r, c, d)).collect();
                if cands.is_empty() {
                    return 0; // dead end, no solutions down this branch
                }
                let singleton = cands.len() == 1;
                if best.as_ref().map_or(true, |x| cands.len() < x.2.len()) {
                    best = Some((r, c, cands));
                }
                if singleton {
                    break 'find;
                }
            }
        }
    }
    let Some((r, c, cands)) = best else {
        return 1; // no empties => one solution
    };
    let mut total = 0;
    for d in cands {
        b[r][c] = d;
        total += count_solutions(b, limit);
        b[r][c] = 0;
        if total >= limit {
            break;
        }
    }
    total
}

// ---------------------------------------------------------------------------
// Puzzle generation (unique, offline)
// ---------------------------------------------------------------------------

/// Minimal xorshift64 PRNG — enough randomness for puzzle generation without a
/// dependency.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng(seed | 1) // xorshift needs a non-zero state
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

/// Fill an empty grid with a random complete, valid solution.
fn fill_grid(b: &mut [[u8; 9]; 9], rng: &mut Rng) -> bool {
    for r in 0..9 {
        for c in 0..9 {
            if b[r][c] == 0 {
                let mut digits: [u8; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];
                rng.shuffle(&mut digits);
                for d in digits {
                    if legal(b, r, c, d) {
                        b[r][c] = d;
                        if fill_grid(b, rng) {
                            return true;
                        }
                        b[r][c] = 0;
                    }
                }
                return false;
            }
        }
    }
    true // no empties left
}

/// Generate a uniquely-solvable puzzle with roughly `target_givens` clues by
/// digging holes out of a random full grid while uniqueness holds.
fn generate_puzzle(target_givens: usize, rng: &mut Rng) -> [[u8; 9]; 9] {
    let mut puzzle = [[0u8; 9]; 9];
    fill_grid(&mut puzzle, rng);

    let mut cells: Vec<(usize, usize)> = (0..81).map(|i| (i / 9, i % 9)).collect();
    rng.shuffle(&mut cells);

    let mut givens = 81;
    for (r, c) in cells {
        if givens <= target_givens {
            break;
        }
        let saved = puzzle[r][c];
        puzzle[r][c] = 0;
        if count_solutions(&mut puzzle.clone(), 2) == 1 {
            givens -= 1;
        } else {
            puzzle[r][c] = saved; // removing it made the puzzle ambiguous
        }
    }
    puzzle
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

impl eframe::App for SudokahApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keyboard(&ctx);
        self.update_timer(&ctx);
        self.update_title(&ctx);

        // Controls live at the bottom (thumb-friendly for Android), with the
        // board filling the remaining space above. When the window is clearly
        // landscape, the board would be squeezed into a short strip, so the
        // controls move to the right instead. The 1.3 ratio keeps near-square
        // windows on the bottom layout.
        let size = ui.available_size();
        if size.x > size.y * 1.3 {
            // Pin the width: the keypad sizes its squares from the panel's
            // available width, so an unpinned panel would grow to fit the
            // squares and starve the board. exact_size breaks that loop.
            egui::Panel::right("toolbar")
                .resizable(false)
                .exact_size((size.x * 0.42).clamp(320.0, 440.0))
                .show(ui, |ui| self.toolbar(ui));
        } else {
            egui::Panel::bottom("toolbar").show(ui, |ui| self.toolbar(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.draw_board(ui);
        });
    }

    /// Persist an in-progress puzzle; clear storage once it's empty or solved.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Best times persist regardless of whether a puzzle is in progress.
        if let Ok(json) = serde_json::to_string(&self.best_times) {
            storage.set_string(BEST_TIMES_KEY, json);
        }
        if self.is_completed() || self.is_empty() {
            storage.set_string(STATE_KEY, String::new());
            return;
        }
        let state = SaveState {
            grid: self.grid,
            baseline: self.baseline,
            set_givens: self.set_givens,
            mode: self.mode,
            show_errors: self.show_errors,
            difficulty: self.difficulty.clone(),
            elapsed_ms: self.elapsed().as_millis() as u64,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            storage.set_string(STATE_KEY, json);
        }
    }
}

impl SudokahApp {
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(4.0);
        let counts = self.digit_counts();
        let done_fill = Color32::from_rgb(120, 190, 130); // a digit fully placed (all 9 on board)
        let spacing = ui.spacing().item_spacing.x;

        // Digit / color pad on a single row: 1-9 plus delete (10 = delete).
        // The cells are taller than wide so the numerals can be drawn large even
        // though ten of them have to share the width.
        let ds = ((ui.available_width() - spacing * 9.0) / 10.0).max(1.0);
        let dh = ds * 1.6;
        let dsz = vec2(ds, dh);
        ui.horizontal(|ui| {
            for d in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10u8] {
                if d == 10 {
                    if ui
                        .add_sized(
                            dsz,
                            egui::Button::new(egui::RichText::new("🗑").size(dh * 0.45)).frame(false),
                        )
                        .on_hover_text("Delete (Backspace)")
                        .clicked()
                    {
                        self.clear_selected();
                    }
                } else if self.mode == Mode::Color {
                    let (rect, resp) = ui.allocate_exact_size(dsz, Sense::click());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(4), COLORS[(d - 1) as usize]);
                    if resp.clicked() {
                        self.apply_digit(d);
                    }
                } else {
                    // Plain digits read as numbers, not buttons; tinted to match the
                    // active mode. A fully-placed digit gets a filled chip (with dark
                    // text for contrast).
                    let done = counts[(d - 1) as usize] == 9;
                    let mut text = egui::RichText::new(format!("{d}")).size(dh * 0.62);
                    if !done {
                        text = text.color(self.mode.ink());
                    }
                    let mut btn = egui::Button::new(text).frame(done);
                    if done {
                        btn = btn.fill(done_fill);
                    }
                    if ui.add_sized(dsz, btn).clicked() {
                        self.apply_digit(d);
                    }
                }
            }
        });

        ui.add_space(4.0);
        // Mode buttons (2x2) share a row with the cursor D-pad. Both blocks are
        // two squares tall so they line up; sized to fill the width as 5 columns
        // (2 mode + 3 D-pad). The arrows are painted as triangles rather than text
        // glyphs because the bundled Android font has no arrow characters (they'd
        // render as empty "tofu" boxes).
        let s = ((ui.available_width() - spacing * 4.0) / 5.0).max(1.0);
        let sz = vec2(s, s);
        let mut nudge: Option<(i32, i32)> = None;
        // `tri` is U/D/L/R; draws a button-styled square with a triangle and
        // reports a click.
        let arrow = |ui: &mut egui::Ui, tri: char| -> bool {
            let (rect, resp) = ui.allocate_exact_size(sz, Sense::click());
            // Soft, recessive look: no frame at rest, a faint highlight only while
            // the key is touched, and a small muted triangle.
            if resp.hovered() || resp.is_pointer_button_down_on() {
                let fill = ui.style().interact(&resp).weak_bg_fill;
                ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
            }
            let c = rect.center();
            let r = s * 0.17;
            let col = ui.visuals().weak_text_color();
            let pts = match tri {
                'U' => vec![pos2(c.x, c.y - r), pos2(c.x - r, c.y + r), pos2(c.x + r, c.y + r)],
                'D' => vec![pos2(c.x, c.y + r), pos2(c.x - r, c.y - r), pos2(c.x + r, c.y - r)],
                'L' => vec![pos2(c.x - r, c.y), pos2(c.x + r, c.y - r), pos2(c.x + r, c.y + r)],
                _ => vec![pos2(c.x + r, c.y), pos2(c.x - r, c.y - r), pos2(c.x - r, c.y + r)],
            };
            ui.painter()
                .add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
            resp.clicked()
        };
        // Same soft style as the arrows, but with a text label (Undo / Redo).
        let soft_btn = |ui: &mut egui::Ui, label: &str| -> bool {
            let (rect, resp) = ui.allocate_exact_size(sz, Sense::click());
            if resp.hovered() || resp.is_pointer_button_down_on() {
                let fill = ui.style().interact(&resp).weak_bg_fill;
                ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
            }
            // Match the mode buttons' label color (the inactive-widget text color).
            let col = ui.visuals().widgets.inactive.fg_stroke.color;
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(s * 0.22),
                col,
            );
            resp.clicked()
        };
        ui.horizontal(|ui| {
            // 2x2 mode buttons.
            ui.vertical(|ui| {
                for row in [
                    [("Digit", "Z", Mode::Normal), ("Corner", "X", Mode::Corner)],
                    [("Center", "C", Mode::Center), ("Color", "V", Mode::Color)],
                ] {
                    ui.horizontal(|ui| {
                        for (label, key, mode) in row {
                            let txt = egui::RichText::new(label).size(s * 0.22);
                            let btn = egui::Button::selectable(self.mode == mode, txt)
                                .stroke(Stroke::new(1.0, Color32::from_gray(140)));
                            if ui
                                .add_sized(sz, btn)
                                .on_hover_text(format!("Shortcut: {key}"))
                                .clicked()
                            {
                                self.mode = mode;
                            }
                        }
                    });
                }
            });
            // D-pad: ↑ on top, ← ↓ → on the bottom row.
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    // Undo / Redo fill the otherwise-empty top corners.
                    if soft_btn(ui, "Undo") {
                        self.undo();
                    }
                    if arrow(ui, 'U') {
                        nudge = Some((-1, 0));
                    }
                    if soft_btn(ui, "Redo") {
                        self.redo();
                    }
                });
                ui.horizontal(|ui| {
                    if arrow(ui, 'L') {
                        nudge = Some((0, -1));
                    }
                    if arrow(ui, 'D') {
                        nudge = Some((1, 0));
                    }
                    if arrow(ui, 'R') {
                        nudge = Some((0, 1));
                    }
                });
            });
        });
        if let Some((dr, dc)) = nudge {
            self.move_cursor(dr, dc, false);
        }

        // One blank row (a button-row tall) separating the cursor block from the
        // buttons below.
        let row_h = ui.spacing().interact_size.y;
        ui.add_space(row_h);
        // egui can't center a sequence of widgets on its own (immediate mode
        // places them left-to-right before the row width is known), so measure
        // each row's content and pad the left edge to center it.
        let item = ui.spacing().item_spacing.x;
        let btn_pad = 2.0 * ui.spacing().button_padding.x;
        let icon_w = ui.spacing().icon_width;
        let icon_gap = ui.spacing().icon_spacing;
        let body_font = egui::TextStyle::Button.resolve(ui.style());
        let big_font = egui::FontId::proportional(22.0);
        let avail = ui.available_width();
        let text_w = |ui: &egui::Ui, t: &str, f: &egui::FontId| -> f32 {
            ui.painter()
                .layout_no_wrap(t.to_owned(), f.clone(), Color32::WHITE)
                .size()
                .x
        };
        let left_pad = |w: f32| ((avail - w) * 0.5).max(0.0);

        // Action row.
        let mut w = 6.0 + item * 4.0; // separator + gaps between the 5 items
        for t in ["🏆 Best times", "Clear marks", "Solve", "New / Clear"] {
            w += text_w(ui, t, &body_font) + btn_pad;
        }
        ui.horizontal(|ui| {
            ui.add_space(left_pad(w));
            if ui.button("🏆 Best times").clicked() {
                self.show_best_times = true;
            }
            if ui.button("Clear marks").clicked() {
                self.clear_pencil_marks();
            }
            ui.separator();
            if ui.button("Solve").clicked() {
                if self.needs_confirm() {
                    self.pending = Some(PendingAction::Solve);
                } else {
                    self.solve();
                }
            }
            if ui.button("New / Clear").clicked() {
                if self.needs_confirm() {
                    self.pending = Some(PendingAction::ClearAll);
                } else {
                    self.clear_all();
                }
            }
        });

        ui.add_space(4.0);
        // Flags row (checkboxes).
        let mut w = item * 2.0; // gaps between the 3 checkboxes
        for t in ["Clues", "Set givens", "Show errors"] {
            w += icon_w + icon_gap + text_w(ui, t, &body_font);
        }
        ui.horizontal(|ui| {
            ui.add_space(left_pad(w));
            ui.checkbox(&mut self.show_auto_candidates, "Clues")
                .on_hover_text("Overlay legal candidates without touching your own marks");
            ui.checkbox(&mut self.set_givens, "Set givens");
            if ui
                .checkbox(&mut self.show_errors, "Show errors")
                .on_hover_text("Highlight digits that don't match the solution in red")
                .changed()
                && self.show_errors
                && self.solution.is_none()
            {
                self.compute_solution();
            }
        });

        ui.add_space(4.0);
        // New-puzzle difficulty buttons.
        let mut w = item * 4.0; // gaps between the 5 buttons
        for t in ["Easy", "Medium", "Hard", "Expert", "Load..."] {
            w += text_w(ui, t, &big_font) + btn_pad;
        }
        ui.horizontal(|ui| {
            ui.add_space(left_pad(w));
            for (label, diff) in [
                ("Easy", "easy"),
                ("Medium", "medium"),
                ("Hard", "hard"),
                ("Expert", "expert"),
            ] {
                if ui
                    .add(egui::Button::new(egui::RichText::new(label).size(22.0)))
                    .clicked()
                {
                    // Guard against accidentally discarding work, but only when the
                    // user has actually changed the board since it loaded.
                    if self.needs_confirm() {
                        self.pending = Some(PendingAction::NewPuzzle(diff.to_owned()));
                    } else {
                        self.new_puzzle(diff);
                    }
                }
            }
            if ui
                .add(egui::Button::new(egui::RichText::new("Load...").size(22.0)))
                .clicked()
            {
                self.show_load_dialog = true;
            }
        });

        // Lift the whole stack up off the bottom edge.
        ui.add_space(100.0);

        self.load_dialog(&ctx);
        self.best_times_dialog(&ctx);
        self.confirm_dialog(&ctx);
    }

    /// Pop-up listing the best solve time for each difficulty, with a button to
    /// wipe them.
    fn best_times_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_best_times {
            return;
        }
        let modal = egui::Modal::new(egui::Id::new("best_times")).show(ctx, |ui| {
            ui.set_width(280.0);
            ui.heading("🏆 Best times");
            ui.add_space(8.0);
            egui::Grid::new("best_times_grid")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    for (label, diff) in [
                        ("Easy", "easy"),
                        ("Medium", "medium"),
                        ("Hard", "hard"),
                        ("Expert", "expert"),
                    ] {
                        ui.label(label);
                        match self.best_times.get(diff) {
                            Some(&secs) => ui.monospace(format_duration(Duration::from_secs(secs))),
                            None => ui.weak("—"),
                        };
                        ui.end_row();
                    }
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Clear best times").clicked() {
                    self.best_times.clear();
                }
                if ui.button("Close").clicked() {
                    self.show_best_times = false;
                }
            });
        });
        // Clicking the dimmed backdrop or pressing Escape also dismisses it.
        if modal.should_close() {
            self.show_best_times = false;
        }
    }

    /// Confirm a destructive action before it discards unfinished work.
    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending.clone() else {
            return;
        };
        let (heading, body, confirm_label) = match &action {
            PendingAction::NewPuzzle(_) => (
                "Start a new puzzle?",
                "Your current puzzle isn't finished and will be replaced.",
                "Replace it",
            ),
            PendingAction::Solve => (
                "Solve the puzzle?",
                "This will fill in the solution and replace your progress.",
                "Solve",
            ),
            PendingAction::ClearAll => (
                "Clear the board?",
                "Your current puzzle isn't finished and will be erased.",
                "Clear",
            ),
        };
        let modal = egui::Modal::new(egui::Id::new("confirm_action")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading(heading);
            ui.add_space(6.0);
            ui.label(body);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(confirm_label).clicked() {
                    match &action {
                        PendingAction::NewPuzzle(diff) => self.new_puzzle(diff),
                        PendingAction::Solve => self.solve(),
                        PendingAction::ClearAll => self.clear_all(),
                    }
                    self.pending = None;
                }
                if ui.button("Cancel").clicked() {
                    self.pending = None;
                }
            });
        });
        // Clicking the dimmed backdrop or pressing Escape cancels.
        if modal.should_close() {
            self.pending = None;
        }
    }

    /// Modal pop-up for pasting an 81-character puzzle string.
    fn load_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_load_dialog {
            return;
        }
        let modal = egui::Modal::new(egui::Id::new("load_dialog")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Load puzzle");
            ui.add_space(6.0);
            ui.label("Paste 81 digits, using 0 or . for blanks:");
            ui.add(
                egui::TextEdit::multiline(&mut self.load_text)
                    .hint_text("81 digits, use 0 or . for blanks")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if !self.load_error.is_empty() {
                ui.add_space(4.0);
                ui.colored_label(Color32::from_rgb(220, 30, 30), &self.load_error);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                // Only close on a successful load; otherwise keep the dialog open
                // so the user can see the error and fix the input.
                if ui.button("Load").clicked() && self.load_from_text() {
                    self.show_load_dialog = false;
                }
                if ui.button("Cancel").clicked() {
                    self.show_load_dialog = false;
                }
            });
        });
        // Clicking the dimmed backdrop or pressing Escape also dismisses it.
        if modal.should_close() {
            self.show_load_dialog = false;
        }
        if !self.show_load_dialog {
            self.load_error.clear();
        }
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        // Only yield keys to an actual text field (the loader). A focused button
        // shouldn't swallow arrow keys / shortcuts.
        if ctx.text_edit_focused() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());

        for ev in events {
            if let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = ev
            {
                // Effective mode: held modifiers temporarily override the button mode.
                let mode = if modifiers.shift {
                    Mode::Corner
                } else if modifiers.command || modifiers.ctrl {
                    Mode::Center
                } else if modifiers.alt {
                    Mode::Color
                } else {
                    self.mode
                };

                if let Some(d) = key_to_digit(key) {
                    let saved = self.mode;
                    self.mode = mode;
                    self.apply_digit(d);
                    self.mode = saved;
                    continue;
                }

                match key {
                    Key::Backspace | Key::Delete => self.clear_selected(),
                    Key::ArrowUp => self.move_cursor(-1, 0, modifiers.shift),
                    Key::ArrowDown => self.move_cursor(1, 0, modifiers.shift),
                    Key::ArrowLeft => self.move_cursor(0, -1, modifiers.shift),
                    Key::ArrowRight => self.move_cursor(0, 1, modifiers.shift),
                    Key::Space => {
                        self.mode = match self.mode {
                            Mode::Normal => Mode::Corner,
                            Mode::Corner => Mode::Center,
                            Mode::Center => Mode::Color,
                            Mode::Color => Mode::Normal,
                        }
                    }
                    Key::Z if modifiers.command || modifiers.ctrl => {
                        if modifiers.shift {
                            self.redo();
                        } else {
                            self.undo();
                        }
                    }
                    Key::Y if modifiers.command || modifiers.ctrl => self.redo(),
                    // Mode keys (when not used as Ctrl shortcuts above).
                    Key::Z => self.mode = Mode::Normal,
                    Key::X => self.mode = Mode::Corner,
                    Key::C => self.mode = Mode::Center,
                    Key::V => self.mode = Mode::Color,
                    Key::Escape => self.selection.clear(),
                    _ => {}
                }
            }
        }
    }

    fn move_cursor(&mut self, dr: i32, dc: i32, extend: bool) {
        let (r, c) = self.selection.last().copied().unwrap_or((0, 0));
        let nr = (r as i32 + dr).rem_euclid(9) as usize;
        let nc = (c as i32 + dc).rem_euclid(9) as usize;
        if !extend {
            self.selection.clear();
        }
        self.select_add((nr, nc));
    }

    fn select_add(&mut self, cell: (usize, usize)) {
        if let Some(pos) = self.selection.iter().position(|&x| x == cell) {
            // Move to end so it acts as the cursor anchor.
            self.selection.remove(pos);
        }
        self.selection.push(cell);
    }

    fn draw_board(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        // Always fill the available width; keep cells as close to square as
        // possible, squishing them vertically only when there isn't enough
        // height for a 1.0 aspect ratio.
        let cw = avail.x.max(200.0) / 9.0;
        let ch = cw.min((avail.y / 9.0).max(1.0));
        let cmin = cw.min(ch); // basis for font sizes
        // Push the board to the bottom of its area so it sits directly above
        // the controls; the leftover height collects as a margin up top.
        ui.add_space((avail.y - ch * 9.0).max(0.0));
        let (rect, response) =
            ui.allocate_exact_size(vec2(cw * 9.0, ch * 9.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let origin = rect.min;

        let cell_at = |p: Pos2| -> Option<(usize, usize)> {
            if !rect.contains(p) {
                return None;
            }
            let c = ((p.x - origin.x) / cw).floor() as i32;
            let r = ((p.y - origin.y) / ch).floor() as i32;
            if (0..9).contains(&r) && (0..9).contains(&c) {
                Some((r as usize, c as usize))
            } else {
                None
            }
        };

        // --- Pointer interaction ---
        let mods = ui.input(|i| i.modifiers);
        if let Some(p) = response.interact_pointer_pos() {
            if let Some(cell) = cell_at(p) {
                if response.drag_started() || response.clicked() {
                    if !(mods.ctrl || mods.command || mods.shift) {
                        self.selection.clear();
                    }
                    self.select_add(cell);
                } else if response.dragged() {
                    if self.selection.last() != Some(&cell) {
                        self.select_add(cell);
                    }
                }
            }
        }

        let cell_rect = |r: usize, c: usize| {
            Rect::from_min_size(
                pos2(origin.x + c as f32 * cw, origin.y + r as f32 * ch),
                vec2(cw, ch),
            )
        };

        // --- Background ---
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);

        // --- Color fills ---
        for r in 0..9 {
            for c in 0..9 {
                if let Some(idx) = self.grid[r][c].color {
                    painter.rect_filled(cell_rect(r, c), CornerRadius::ZERO, COLORS[idx]);
                }
            }
        }

        // --- Selection highlight ---
        // Light wash over the row, column, and block of every selected cell, so
        // the selection's peers stand out. A set avoids painting overlaps twice
        // (which would stack the alpha and darken shared cells).
        let mut peers = std::collections::HashSet::new();
        for &(sr, sc) in &self.selection {
            let (br, bc) = (sr - sr % 3, sc - sc % 3);
            for i in 0..9 {
                peers.insert((sr, i));
                peers.insert((i, sc));
                peers.insert((br + i / 3, bc + i % 3));
            }
        }
        let peer = Color32::from_rgba_unmultiplied(90, 140, 250, 28);
        for &(r, c) in &peers {
            if !self.selection.contains(&(r, c)) {
                painter.rect_filled(cell_rect(r, c), CornerRadius::ZERO, peer);
            }
        }
        let sel = Color32::from_rgba_unmultiplied(90, 140, 250, 90);
        for &(r, c) in &self.selection {
            painter.rect_filled(cell_rect(r, c), CornerRadius::ZERO, sel);
        }

        // --- Digits & pencil marks ---
        let given_col = Color32::from_rgb(20, 20, 20);
        let user_col = USER_COL;
        let center_col = CENTER_COL; // candidates (center marks): navy
        let corner_col = CORNER_COL; // corner marks: brown
        let auto_col = Color32::from_rgb(80, 92, 140); // auto candidates overlay: muted blue-grey
        let error_col = Color32::from_rgb(220, 30, 30); // wrong digit (Show errors): red
        for r in 0..9 {
            for c in 0..9 {
                let cr = cell_rect(r, c);
                let cell = &self.grid[r][c];
                if let Some(d) = cell.value {
                    // A user digit is "wrong" when Show errors is on and it
                    // doesn't match the cached solution.
                    let wrong = self.show_errors
                        && !cell.given
                        && self.solution.is_some_and(|sol| sol[r][c] != d);
                    let col = if cell.given {
                        given_col
                    } else if wrong {
                        error_col
                    } else {
                        user_col
                    };
                    painter.text(
                        cr.center(),
                        Align2::CENTER_CENTER,
                        d.to_string(),
                        FontId::proportional(cmin * 0.62),
                        col,
                    );
                } else {
                    // Center marks: the user's own, or — for cells the user
                    // hasn't marked — the auto-candidate overlay (which never
                    // touches the stored marks, so it toggles cleanly off).
                    let has_user_center = cell.center.iter().any(|&x| x);
                    let (marks, mark_col) = if has_user_center {
                        (cell.center, center_col)
                    } else if self.show_auto_candidates {
                        (self.cell_candidates(r, c), auto_col)
                    } else {
                        ([false; 9], center_col)
                    };
                    let digits: String = (0..9)
                        .filter(|&i| marks[i])
                        .map(|i| char::from(b'1' + i as u8))
                        .collect();
                    if !digits.is_empty() {
                        let n = digits.len();
                        let fs = (cmin * 0.24).min(cmin * 1.7 / n as f32).max(cmin * 0.16);
                        painter.text(
                            cr.center(),
                            Align2::CENTER_CENTER,
                            digits,
                            FontId::proportional(fs),
                            mark_col,
                        );
                    }
                    // Corner marks
                    let corner_slots = [
                        (0.20, 0.20),
                        (0.80, 0.20),
                        (0.20, 0.80),
                        (0.80, 0.80),
                        (0.50, 0.20),
                        (0.50, 0.80),
                        (0.20, 0.50),
                        (0.80, 0.50),
                        (0.50, 0.50),
                    ];
                    let active: Vec<usize> = (0..9).filter(|&i| cell.corner[i]).collect();
                    for (slot, &i) in active.iter().enumerate() {
                        let (fx, fy) = corner_slots[slot.min(8)];
                        let p = pos2(cr.min.x + fx * cw, cr.min.y + fy * ch);
                        painter.text(
                            p,
                            Align2::CENTER_CENTER,
                            char::from(b'1' + i as u8).to_string(),
                            FontId::proportional(cmin * 0.22),
                            corner_col,
                        );
                    }
                }
            }
        }

        // --- Grid lines ---
        let thin = Stroke::new(1.0, Color32::from_gray(170));
        let thick = Stroke::new(2.5, Color32::from_gray(30));
        for i in 0..=9 {
            let stroke = if i % 3 == 0 { thick } else { thin };
            let x = origin.x + i as f32 * cw;
            let y = origin.y + i as f32 * ch;
            painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], stroke);
            painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
        }

        // --- Solved indicator ---
        if self.is_completed() {
            let green = Color32::from_rgb(40, 170, 70);
            painter.rect_stroke(
                rect,
                CornerRadius::ZERO,
                Stroke::new(6.0, green),
                egui::StrokeKind::Inside,
            );
            let banner = Rect::from_center_size(rect.center(), vec2(rect.width(), cmin * 1.7));
            painter.rect_filled(
                banner,
                CornerRadius::ZERO,
                Color32::from_rgba_unmultiplied(255, 255, 255, 230),
            );
            let center = rect.center();
            painter.text(
                pos2(center.x, center.y - cmin * 0.45),
                Align2::CENTER_CENTER,
                "Solved! 🎉",
                FontId::proportional(cmin * 0.62),
                green,
            );
            painter.text(
                pos2(center.x, center.y + cmin * 0.2),
                Align2::CENTER_CENTER,
                format!("Time {}", format_duration(self.elapsed())),
                FontId::proportional(cmin * 0.32),
                Color32::from_rgb(40, 40, 40),
            );
            if self.new_best {
                painter.text(
                    pos2(center.x, center.y + cmin * 0.62),
                    Align2::CENTER_CENTER,
                    "⭐ New best time!",
                    FontId::proportional(cmin * 0.3),
                    Color32::from_rgb(200, 140, 0),
                );
            }
        }
    }
}

fn key_to_digit(key: Key) -> Option<u8> {
    Some(match key {
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_known_puzzle() {
        // A puzzle with a unique solution (81 chars).
        let s = "530070000600195000098000060800060003400803001700020006060000280000419005000080079";
        assert_eq!(s.len(), 81);
        let mut b = [[0u8; 9]; 9];
        for (i, ch) in s.chars().enumerate() {
            b[i / 9][i % 9] = ch.to_digit(10).unwrap_or(0) as u8;
        }
        assert!(board_consistent(&b));
        assert!(backtrack(&mut b));
        // Verify it's a valid completed grid.
        for r in 0..9 {
            for c in 0..9 {
                assert!(b[r][c] != 0);
                let d = b[r][c];
                b[r][c] = 0;
                assert!(legal(&b, r, c, d), "conflict at {r},{c}");
                b[r][c] = d;
            }
        }
        assert_eq!(b[0][2], 4); // known first-row solution digit
    }

    #[test]
    fn generates_unique_puzzles() {
        let mut rng = Rng::new();
        for target in [40usize, 32, 26] {
            let puzzle = generate_puzzle(target, &mut rng);
            let givens = puzzle.iter().flatten().filter(|&&d| d != 0).count();
            assert!(givens >= target, "fewer givens than asked: {givens} < {target}");
            assert!(board_consistent(&puzzle));
            assert_eq!(count_solutions(&mut puzzle.clone(), 2), 1, "not unique");
        }
    }

    #[test]
    fn best_time_tracked_per_difficulty() {
        let mut app = SudokahApp::default();

        // First solve of a difficulty is recorded but not celebrated.
        app.difficulty = Some("easy".into());
        app.timer_elapsed = Duration::from_secs(120);
        app.record_solve();
        assert_eq!(app.best_times.get("easy"), Some(&120));
        assert!(!app.new_best);

        // A faster solve beats the record and is celebrated.
        app.new_best = false;
        app.timer_elapsed = Duration::from_secs(90);
        app.record_solve();
        assert_eq!(app.best_times.get("easy"), Some(&90));
        assert!(app.new_best);

        // A slower solve leaves the record (and flag) untouched.
        app.new_best = false;
        app.timer_elapsed = Duration::from_secs(150);
        app.record_solve();
        assert_eq!(app.best_times.get("easy"), Some(&90));
        assert!(!app.new_best);

        // Difficulties are tracked independently.
        app.difficulty = Some("hard".into());
        app.timer_elapsed = Duration::from_secs(300);
        app.record_solve();
        assert_eq!(app.best_times.get("hard"), Some(&300));
        assert_eq!(app.best_times.get("easy"), Some(&90));

        // Hand-loaded puzzles (no difficulty) record nothing.
        app.difficulty = None;
        app.new_best = false;
        app.timer_elapsed = Duration::from_secs(5);
        app.record_solve();
        assert_eq!(app.best_times.len(), 2);
        assert!(!app.new_best);

        // A board finished by the solver earns no best, even if faster.
        app.difficulty = Some("easy".into());
        app.solver_used = true;
        app.new_best = false;
        app.timer_elapsed = Duration::from_secs(1);
        app.record_solve();
        assert_eq!(app.best_times.get("easy"), Some(&90));
        assert!(!app.new_best);
    }

    #[test]
    fn solve_button_does_not_set_best() {
        // Drive a generated puzzle to completion via the solver and confirm no
        // best time is banked.
        let mut app = SudokahApp::default();
        let mut rng = Rng::new();
        let board = generate_puzzle(40, &mut rng);
        app.set_board_givens(&board);
        app.start_timer(Some("easy".into()));
        app.solve();
        assert!(app.solver_used);
        assert!(app.is_completed());
        assert!(app.best_times.is_empty());
        // A redundant completion check must still record nothing.
        app.record_solve();
        assert!(app.best_times.is_empty());
    }
}
