//! A display-independent, fixed-step Snake model with injected randomness.

use oxiview_base::Rng;
use std::{
    collections::{HashSet, VecDeque},
    time::Duration,
};

/// A cell in the board's integer coordinate space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// Zero-based horizontal position.
    pub x: u16,
    /// Zero-based vertical position.
    pub y: u16,
}

/// Validated board dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Board {
    columns: u16,
    rows: u16,
}
impl Board {
    /// Creates a board with each dimension in 4..=64.
    pub fn new(columns: u16, rows: u16) -> Result<Self, Error> {
        if !(4..=64).contains(&columns) || !(4..=64).contains(&rows) {
            return Err(Error::InvalidBoard);
        }
        Ok(Self { columns, rows })
    }
    /// Returns the number of columns.
    pub fn columns(self) -> u16 {
        self.columns
    }
    /// Returns the number of rows.
    pub fn rows(self) -> u16 {
        self.rows
    }
    /// Returns the cell capacity.
    pub fn capacity(self) -> usize {
        usize::from(self.columns) * usize::from(self.rows)
    }
    /// Whether a cell belongs to this board.
    pub fn contains(self, cell: Cell) -> bool {
        cell.x < self.columns && cell.y < self.rows
    }
}
impl Default for Board {
    fn default() -> Self {
        Self {
            columns: 24,
            rows: 20,
        }
    }
}

/// A cardinal movement direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Toward the top edge.
    Up,
    /// Toward the right edge.
    Right,
    /// Toward the bottom edge.
    Down,
    /// Toward the left edge.
    Left,
}
impl Direction {
    /// Returns the reverse direction.
    pub fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Right => Self::Left,
            Self::Down => Self::Up,
            Self::Left => Self::Right,
        }
    }
    /// Returns the next cell, or none when the board edge would be crossed.
    pub fn step(self, cell: Cell, board: Board) -> Option<Cell> {
        let (dx, dy) = match self {
            Self::Up => (0, -1),
            Self::Right => (1, 0),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
        };
        let next = Cell {
            x: cell.x.checked_add_signed(dx)?,
            y: cell.y.checked_add_signed(dy)?,
        };
        board.contains(next).then_some(next)
    }
}

/// The starting movement cadence; later levels gently increase speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Difficulty {
    /// A relaxed 180 ms starting interval.
    Chill,
    /// A 125 ms starting interval.
    #[default]
    Classic,
    /// A faster 90 ms starting interval.
    Swift,
}
impl Difficulty {
    /// Human-readable choice label.
    pub fn name(self) -> &'static str {
        match self {
            Self::Chill => "Chill",
            Self::Classic => "Classic",
            Self::Swift => "Swift",
        }
    }
    fn initial_millis(self) -> u64 {
        match self {
            Self::Chill => 180,
            Self::Classic => 125,
            Self::Swift => 90,
        }
    }
}

/// Why a run ended without filling the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collision {
    /// The head crossed a board boundary.
    Wall,
    /// The head intersected a segment that would remain occupied.
    Body,
}
/// The game's visible lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for an explicit start.
    Ready,
    /// Advancing on scheduled ticks.
    Running,
    /// Suspended without a periodic frame request.
    Paused,
    /// A collision ended the run.
    Over(Collision),
    /// Every board cell is occupied by the snake.
    Won,
}
/// An immutable value suitable for an oxiview Signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Board dimensions.
    pub board: Board,
    /// Head-first occupied cells.
    pub body: Vec<Cell>,
    /// Food, absent only when the board is full.
    pub food: Option<Cell>,
    /// Direction of the last applied movement.
    pub direction: Direction,
    /// Current game lifecycle.
    pub phase: Phase,
    /// Ten points per collected food.
    pub score: u32,
    /// Best score within this application session.
    pub best: u32,
    /// One-based speed level.
    pub level: u32,
    /// Selected starting cadence.
    pub difficulty: Difficulty,
    /// Total attempted movement ticks in the current run.
    pub ticks: u64,
}
/// The result of a turn request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnResult {
    /// Accepted for a future tick.
    Queued,
    /// Already moving or queued in this direction.
    Duplicate,
    /// Would reverse the immediately preceding direction.
    Reverse,
    /// Two distinct turns are already waiting.
    QueueFull,
    /// The game is not running.
    Inactive,
}
/// The result of one simulation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickResult {
    /// A normal move with unchanged length.
    Moved,
    /// Food was consumed and the snake grew.
    Ate,
    /// The move collided and ended the run.
    Collision(Collision),
    /// The final free cell was occupied.
    Won,
    /// No simulation runs in the current phase.
    Idle,
}
/// A recoverable game setup or random-source failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The example deliberately bounds board dimensions.
    #[error("board dimensions must each be in 4..=64")]
    InvalidBoard,
    /// A caller-supplied random source failed.
    #[error("food random source failed: {0}")]
    Random(#[from] oxiview_base::Error),
    /// Internal model state cannot describe a valid run.
    #[error("invalid game state: {0}")]
    InvalidState(&'static str),
}

/// A reproducible non-cryptographic source for game food placement.
#[derive(Clone, Debug)]
pub struct SeededRng {
    state: u64,
}
impl SeededRng {
    /// Creates a stream; zero is mapped to a fixed nonzero seed.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }
}
impl Rng for SeededRng {
    fn next_u64(&mut self) -> Result<u64, oxiview_base::Error> {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        Ok(self.state)
    }
}

/// Snake rules independent of UI nodes, the display clock, and native input.
pub struct Game<R: Rng> {
    board: Board,
    body: VecDeque<Cell>,
    food: Option<Cell>,
    direction: Direction,
    turns: VecDeque<Direction>,
    phase: Phase,
    score: u32,
    best: u32,
    difficulty: Difficulty,
    ticks: u64,
    rng: R,
}
impl<R: Rng> Game<R> {
    /// Creates a ready game with four horizontal segments.
    pub fn new(board: Board, difficulty: Difficulty, mut rng: R) -> Result<Self, Error> {
        let body = initial_body(board);
        let food = choose_food(board, &body, &mut rng)?;
        Ok(Self {
            board,
            body,
            food,
            direction: Direction::Right,
            turns: VecDeque::new(),
            phase: Phase::Ready,
            score: 0,
            best: 0,
            difficulty,
            ticks: 0,
            rng,
        })
    }
    /// Returns a stable, owned snapshot for UI bindings and replay assertions.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            board: self.board,
            body: self.body.iter().copied().collect(),
            food: self.food,
            direction: self.direction,
            phase: self.phase,
            score: self.score,
            best: self.best,
            level: self.level(),
            difficulty: self.difficulty,
            ticks: self.ticks,
        }
    }
    /// Returns the current lifecycle phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }
    /// Returns the current fixed simulation interval.
    pub fn tick_interval(&self) -> Duration {
        let reduction = u64::from(self.level().saturating_sub(1)) * 8;
        Duration::from_millis(
            self.difficulty
                .initial_millis()
                .saturating_sub(reduction)
                .max(55),
        )
    }
    fn level(&self) -> u32 {
        self.score / 50 + 1
    }
    /// Starts a ready game or resumes a paused game.
    pub fn start(&mut self) -> bool {
        if matches!(self.phase, Phase::Ready | Phase::Paused) {
            self.phase = Phase::Running;
            true
        } else {
            false
        }
    }
    /// Pauses only an active run, discarding unconsumed turns.
    pub fn pause(&mut self) -> bool {
        if self.phase == Phase::Running {
            self.phase = Phase::Paused;
            self.turns.clear();
            true
        } else {
            false
        }
    }
    /// Resets a run while retaining this session's best score.
    pub fn restart(&mut self, start: bool) -> Result<(), Error> {
        let body = initial_body(self.board);
        let food = choose_food(self.board, &body, &mut self.rng)?;
        self.body = body;
        self.food = food;
        self.direction = Direction::Right;
        self.turns.clear();
        self.score = 0;
        self.ticks = 0;
        self.phase = if start { Phase::Running } else { Phase::Ready };
        Ok(())
    }
    /// Selects a difficulty while ready or after a finished run.
    pub fn set_difficulty(&mut self, difficulty: Difficulty) -> bool {
        if matches!(self.phase, Phase::Running | Phase::Paused) || self.difficulty == difficulty {
            return false;
        }
        self.difficulty = difficulty;
        true
    }
    /// Queues at most two turns, checking reversals against the queued path.
    pub fn turn(&mut self, direction: Direction) -> TurnResult {
        if self.phase != Phase::Running {
            return TurnResult::Inactive;
        }
        let preceding = self.turns.back().copied().unwrap_or(self.direction);
        if direction == preceding {
            return TurnResult::Duplicate;
        }
        if direction == preceding.opposite() {
            return TurnResult::Reverse;
        }
        if self.turns.len() >= 2 {
            return TurnResult::QueueFull;
        }
        self.turns.push_back(direction);
        TurnResult::Queued
    }
    /// Applies one fixed step. Random-source failure leaves model state intact.
    pub fn tick(&mut self) -> Result<TickResult, Error> {
        if self.phase != Phase::Running {
            return Ok(TickResult::Idle);
        }
        let head = self
            .body
            .front()
            .copied()
            .ok_or(Error::InvalidState("missing head"))?;
        let direction = self.turns.front().copied().unwrap_or(self.direction);
        let Some(next) = direction.step(head, self.board) else {
            return Ok(self.collide(Collision::Wall, direction));
        };
        let grows = self.food == Some(next);
        let moves_into_tail = self.body.back() == Some(&next) && !grows;
        if self.body.contains(&next) && !moves_into_tail {
            return Ok(self.collide(Collision::Body, direction));
        }

        let mut body = self.body.clone();
        body.push_front(next);
        if !grows {
            body.pop_back();
        }
        let (food, outcome) = if grows {
            if body.len() == self.board.capacity() {
                (None, TickResult::Won)
            } else {
                (
                    choose_food(self.board, &body, &mut self.rng)?,
                    TickResult::Ate,
                )
            }
        } else {
            (self.food, TickResult::Moved)
        };

        self.body = body;
        self.food = food;
        self.direction = direction;
        self.turns.pop_front();
        self.ticks = self.ticks.saturating_add(1);
        if grows {
            self.score = self.score.saturating_add(10);
            self.best = self.best.max(self.score);
        }
        if outcome == TickResult::Won {
            self.phase = Phase::Won;
            self.turns.clear();
        }
        Ok(outcome)
    }
    fn collide(&mut self, collision: Collision, direction: Direction) -> TickResult {
        self.direction = direction;
        self.phase = Phase::Over(collision);
        self.turns.clear();
        self.ticks = self.ticks.saturating_add(1);
        TickResult::Collision(collision)
    }
}

fn initial_body(board: Board) -> VecDeque<Cell> {
    let x = (board.columns / 2).max(3);
    (0..4)
        .map(|offset| Cell {
            x: x - offset,
            y: board.rows / 2,
        })
        .collect()
}
fn choose_food<R: Rng>(
    board: Board,
    body: &VecDeque<Cell>,
    rng: &mut R,
) -> Result<Option<Cell>, Error> {
    let occupied: HashSet<_> = body.iter().copied().collect();
    let free: Vec<_> = (0..board.rows)
        .flat_map(|y| (0..board.columns).map(move |x| Cell { x, y }))
        .filter(|cell| !occupied.contains(cell))
        .collect();
    if free.is_empty() {
        return Ok(None);
    }
    let count =
        u64::try_from(free.len()).map_err(|_| Error::InvalidState("food capacity conversion"))?;
    let index = usize::try_from(rng.next_u64()? % count)
        .map_err(|_| Error::InvalidState("food index conversion"))?;
    free.get(index)
        .copied()
        .map(Some)
        .ok_or(Error::InvalidState("food index missing"))
}

/// The result of advancing a game clock, including display-stall protection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Advance {
    /// Number of simulation steps applied.
    pub ticks: u8,
    /// Whether a large timing gap paused the game instead of fast-forwarding it.
    pub paused_for_delay: bool,
}
/// An explicit clock that schedules nothing while a game is idle or paused.
#[derive(Default, Debug)]
pub struct TickClock {
    next: Option<Duration>,
    last: Option<Duration>,
}
impl TickClock {
    /// Arms the first tick from an explicit monotonic timestamp.
    pub fn start(&mut self, now: Duration, interval: Duration) {
        self.next = Some(now.saturating_add(interval));
        self.last = Some(now);
    }
    /// Cancels all periodic work.
    pub fn cancel(&mut self) {
        self.next = None;
        self.last = None;
    }
    /// Returns the one next deadline, or none when no tick is scheduled.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.next
    }
    /// Applies at most four due ticks. A long gap pauses instead of killing an unseen run.
    pub fn advance<R: Rng>(&mut self, now: Duration, game: &mut Game<R>) -> Result<Advance, Error> {
        if game.phase() != Phase::Running {
            self.cancel();
            return Ok(Advance::default());
        }
        if self.last.is_some_and(|last| now < last) || self.next.is_none() {
            self.start(now, game.tick_interval());
            return Ok(Advance::default());
        }
        self.last = Some(now);
        let Some(deadline) = self.next else {
            return Ok(Advance::default());
        };
        if now.saturating_sub(deadline) > Duration::from_millis(500) {
            game.pause();
            self.cancel();
            return Ok(Advance {
                ticks: 0,
                paused_for_delay: true,
            });
        }
        let mut ticks = 0;
        while ticks < 4 && self.next.is_some_and(|deadline| deadline <= now) {
            let Some(deadline) = self.next else {
                break;
            };
            game.tick()?;
            ticks += 1;
            if game.phase() != Phase::Running {
                self.cancel();
                break;
            }
            self.next = Some(deadline.saturating_add(game.tick_interval()));
        }
        let paused_for_delay = self.next.is_some_and(|deadline| deadline <= now);
        if paused_for_delay {
            game.pause();
            self.cancel();
        }
        Ok(Advance {
            ticks,
            paused_for_delay,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiview_base::ManualRng;
    use proptest::prelude::*;

    fn seeded() -> Result<Game<SeededRng>, Error> {
        Game::new(Board::default(), Difficulty::Classic, SeededRng::new(42))
    }
    fn assert_valid(snapshot: &Snapshot) {
        let occupied: HashSet<_> = snapshot.body.iter().copied().collect();
        assert_eq!(occupied.len(), snapshot.body.len());
        assert!(
            snapshot
                .body
                .iter()
                .all(|cell| snapshot.board.contains(*cell))
        );
        assert!(
            snapshot
                .food
                .is_none_or(|food| snapshot.board.contains(food) && !occupied.contains(&food))
        );
        assert!(snapshot.best >= snapshot.score);
    }
    #[test]
    fn ready_and_paused_games_do_not_advance() -> Result<(), Error> {
        let mut game = seeded()?;
        let ready = game.snapshot();
        assert_eq!(game.tick()?, TickResult::Idle);
        assert_eq!(game.snapshot(), ready);
        assert!(game.start());
        game.tick()?;
        assert!(game.pause());
        let paused = game.snapshot();
        for _ in 0..1000 {
            assert_eq!(game.tick()?, TickResult::Idle);
        }
        assert_eq!(game.snapshot(), paused);
        Ok(())
    }
    #[test]
    fn queued_turns_cannot_reverse_between_ticks() -> Result<(), Error> {
        let mut game = seeded()?;
        game.start();
        assert_eq!(game.turn(Direction::Left), TurnResult::Reverse);
        assert_eq!(game.turn(Direction::Up), TurnResult::Queued);
        assert_eq!(game.turn(Direction::Down), TurnResult::Reverse);
        assert_eq!(game.turn(Direction::Left), TurnResult::Queued);
        assert_eq!(game.turn(Direction::Down), TurnResult::QueueFull);
        game.tick()?;
        assert_eq!(game.snapshot().direction, Direction::Up);
        game.tick()?;
        assert_eq!(game.snapshot().direction, Direction::Left);
        assert_valid(&game.snapshot());
        Ok(())
    }
    #[test]
    fn eating_grows_and_allocates_food_only_in_free_cells() -> Result<(), Error> {
        let board = Board::new(6, 6)?;
        // Head is (3,3); four occupied cells precede the desired food (4,3).
        let mut game = Game::new(board, Difficulty::Classic, ManualRng::new([18, 0]))?;
        assert_eq!(game.food, Some(Cell { x: 4, y: 3 }));
        game.start();
        assert_eq!(game.tick()?, TickResult::Ate);
        assert_eq!(game.snapshot().body.len(), 5);
        assert_eq!(game.snapshot().score, 10);
        assert_eq!(game.snapshot().best, 10);
        assert_valid(&game.snapshot());
        Ok(())
    }
    #[test]
    fn failed_food_generation_does_not_partially_advance() -> Result<(), Error> {
        let mut game = Game::new(Board::new(6, 6)?, Difficulty::Classic, ManualRng::new([18]))?;
        game.start();
        let before = game.snapshot();
        assert!(matches!(game.tick(), Err(Error::Random(_))));
        assert_eq!(game.snapshot(), before);
        Ok(())
    }
    #[test]
    fn moving_into_the_vacating_tail_is_legal() -> Result<(), Error> {
        let mut game = seeded()?;
        game.body = [
            Cell { x: 3, y: 3 },
            Cell { x: 3, y: 4 },
            Cell { x: 2, y: 4 },
            Cell { x: 2, y: 3 },
        ]
        .into();
        game.direction = Direction::Up;
        game.food = Some(Cell { x: 0, y: 0 });
        game.start();
        assert_eq!(game.turn(Direction::Left), TurnResult::Queued);
        assert_eq!(game.tick()?, TickResult::Moved);
        assert_eq!(game.body.front(), Some(&Cell { x: 2, y: 3 }));
        assert_valid(&game.snapshot());
        Ok(())
    }
    #[test]
    fn body_and_wall_collisions_leave_a_valid_terminal_board() -> Result<(), Error> {
        let mut game = seeded()?;
        game.body = [
            Cell { x: 3, y: 3 },
            Cell { x: 3, y: 4 },
            Cell { x: 2, y: 4 },
            Cell { x: 2, y: 3 },
            Cell { x: 2, y: 2 },
        ]
        .into();
        game.direction = Direction::Up;
        game.food = Some(Cell { x: 0, y: 0 });
        game.start();
        game.turn(Direction::Left);
        assert_eq!(game.tick()?, TickResult::Collision(Collision::Body));
        assert_eq!(game.tick()?, TickResult::Idle);
        assert_valid(&game.snapshot());
        game.restart(true)?;
        for _ in 0..64 {
            if matches!(game.tick()?, TickResult::Collision(_)) {
                break;
            }
        }
        assert_eq!(game.phase(), Phase::Over(Collision::Wall));
        assert_valid(&game.snapshot());
        Ok(())
    }
    #[test]
    fn a_full_board_wins_without_requesting_more_randomness() -> Result<(), Error> {
        let board = Board::new(4, 4)?;
        let mut game = Game::new(board, Difficulty::Classic, ManualRng::new([0]))?;
        game.body = [
            (1, 0),
            (2, 0),
            (3, 0),
            (3, 1),
            (2, 1),
            (1, 1),
            (0, 1),
            (0, 2),
            (1, 2),
            (2, 2),
            (3, 2),
            (3, 3),
            (2, 3),
            (1, 3),
            (0, 3),
        ]
        .into_iter()
        .map(|(x, y)| Cell { x, y })
        .collect();
        game.food = Some(Cell { x: 0, y: 0 });
        game.direction = Direction::Left;
        game.start();
        assert_eq!(game.tick()?, TickResult::Won);
        assert_eq!(game.phase(), Phase::Won);
        assert_eq!(game.body.len(), board.capacity());
        assert_eq!(game.food, None);
        assert_eq!(game.tick()?, TickResult::Idle);
        Ok(())
    }
    #[test]
    fn restart_preserves_best_and_difficulty_changes_only_when_idle() -> Result<(), Error> {
        let mut game = Game::new(
            Board::new(6, 6)?,
            Difficulty::Classic,
            ManualRng::new([18, 0, 4]),
        )?;
        assert!(game.set_difficulty(Difficulty::Chill));
        game.start();
        game.tick()?;
        assert!(!game.set_difficulty(Difficulty::Swift));
        game.restart(false)?;
        let snapshot = game.snapshot();
        assert_eq!(snapshot.phase, Phase::Ready);
        assert_eq!(snapshot.score, 0);
        assert_eq!(snapshot.best, 10);
        assert_eq!(snapshot.body.len(), 4);
        assert_eq!(snapshot.direction, Direction::Right);
        assert_eq!(snapshot.difficulty, Difficulty::Chill);
        Ok(())
    }
    #[test]
    fn clock_stops_on_pause_and_avoids_long_gap_fast_forward() -> Result<(), Error> {
        let mut game = seeded()?;
        let mut clock = TickClock::default();
        assert_eq!(
            clock.advance(Duration::from_secs(100), &mut game)?,
            Advance::default()
        );
        assert_eq!(clock.next_deadline(), None);
        game.start();
        clock.start(Duration::from_secs(1), game.tick_interval());
        assert_eq!(
            clock.advance(Duration::from_millis(1124), &mut game)?.ticks,
            0
        );
        assert_eq!(
            clock.advance(Duration::from_millis(1125), &mut game)?.ticks,
            1
        );
        let before = game.snapshot().body;
        assert!(
            clock
                .advance(Duration::from_secs(60), &mut game)?
                .paused_for_delay
        );
        assert_eq!(game.phase(), Phase::Paused);
        assert_eq!(game.snapshot().body, before);
        assert_eq!(clock.next_deadline(), None);
        for now in 61..1061 {
            assert_eq!(
                clock.advance(Duration::from_secs(now), &mut game)?,
                Advance::default()
            );
        }
        Ok(())
    }
    #[test]
    fn same_seed_and_inputs_replay_identically() -> Result<(), Error> {
        let mut a = seeded()?;
        let mut b = seeded()?;
        a.start();
        b.start();
        for direction in [
            Direction::Up,
            Direction::Left,
            Direction::Down,
            Direction::Right,
        ]
        .into_iter()
        .cycle()
        .take(100)
        {
            assert_eq!(a.turn(direction), b.turn(direction));
            assert_eq!(a.tick()?, b.tick()?);
            assert_eq!(a.snapshot(), b.snapshot());
        }
        Ok(())
    }
    proptest! {
        #[test]
        fn arbitrary_turn_sequences_preserve_occupancy(seed in any::<u64>(), inputs in prop::collection::vec(0u8..8, 0..300)) {
            let mut game = Game::new(Board::default(), Difficulty::Classic, SeededRng::new(seed)).map_err(|e| TestCaseError::fail(e.to_string()))?;
            game.start();
            for input in inputs {
                let direction = match input % 4 { 0 => Direction::Up, 1 => Direction::Right, 2 => Direction::Down, _ => Direction::Left };
                game.turn(direction);
                if input < 4 { game.tick().map_err(|e| TestCaseError::fail(e.to_string()))?; }
                assert_valid(&game.snapshot());
            }
        }
    }
}
