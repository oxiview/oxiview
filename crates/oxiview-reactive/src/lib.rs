//! Thread-local fine-grained signals, derived values, effects, and owning scopes.
//!
//! Handles are cheap copies, but their scope owns the resources. Reads and writes
//! reject stale handles and conflicting borrows. Derived computations settle before
//! observing effects; batches delay observers until their final input values exist.

use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeSet, VecDeque},
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

/// Maximum callbacks evaluated by one synchronous flush.
pub const EFFECT_BUDGET: usize = 4096;
const MAX_EFFECT_DEPTH: usize = 64;
static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);

mod executor;
pub use executor::{
    BackgroundError, BackgroundPool, BackgroundTask, LocalExecutor, LocalTask, PollReport,
    spawn_background, spawn_local,
};

/// Checked reactive lifetime, borrowing, and evaluation failures.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReactiveError {
    /// A scope, signal, or effect has been disposed.
    #[error("reactive handle has been disposed")]
    Disposed,
    /// A scope owner belongs to a different thread-local runtime.
    #[error("scope owner belongs to a different reactive runtime")]
    ForeignRuntime,
    /// The thread's implicit root scope cannot be adopted.
    #[error("the reactive root scope cannot be adopted")]
    RootScope,
    /// Adoption would create a scope ownership cycle.
    #[error("scope adoption would create an ownership cycle")]
    ScopeCycle,
    /// A moved scope or descendant is currently executing a callback.
    #[error("an executing scope subtree cannot be adopted")]
    ActiveScope,
    /// A callback attempted to borrow a value incompatibly or reenter bookkeeping.
    #[error("reactive value is already borrowed")]
    BorrowConflict,
    /// A derived computation depends on itself.
    #[error("cyclic reactive computation")]
    Cycle,
    /// Callbacks did not settle within the bounded synchronous budget.
    #[error("reactive effect budget exceeded")]
    UpdateLimit,
    /// A derived value is unavailable before its computation succeeds.
    #[error("derived value has not been initialized")]
    Uninitialized,
    /// Runtime access occurred while thread-local storage was being destroyed.
    #[error("reactive runtime is unavailable")]
    RuntimeUnavailable,
    /// Identity or batching counters cannot advance safely.
    #[error("reactive identity or nesting counter exhausted")]
    Exhausted,
    /// The handle's stored value has an unexpected type.
    #[error("reactive value type mismatch")]
    TypeMismatch,
    /// No local executor is entered on this thread, or it has closed.
    #[error("no active local executor")]
    ExecutorUnavailable,
    /// An executor was polled recursively.
    #[error("local executor is already being polled")]
    ReentrantPoll,
    /// An executor poll budget must be positive.
    #[error("local executor poll budget must be positive")]
    InvalidBudget,
    /// A binding or application computation reported an error.
    #[error("reactive callback failed: {0}")]
    User(String),
}

/// A reactive operation result.
pub type Result<T> = std::result::Result<T, ReactiveError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    index: usize,
    generation: u64,
}
struct Slot<T> {
    generation: u64,
    value: Option<T>,
}
struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<usize>,
}
impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}
impl<T> Arena<T> {
    fn insert(&mut self, value: T) -> Key {
        while let Some(index) = self.free.pop() {
            if let Some(slot) = self.slots.get_mut(index)
                && slot.value.is_none()
            {
                slot.value = Some(value);
                return Key {
                    index,
                    generation: slot.generation,
                };
            }
        }
        let index = self.slots.len();
        self.slots.push(Slot {
            generation: 1,
            value: Some(value),
        });
        Key {
            index,
            generation: 1,
        }
    }
    fn get(&self, key: Key) -> Result<&T> {
        self.slots
            .get(key.index)
            .filter(|slot| slot.generation == key.generation)
            .and_then(|slot| slot.value.as_ref())
            .ok_or(ReactiveError::Disposed)
    }
    fn get_mut(&mut self, key: Key) -> Result<&mut T> {
        self.slots
            .get_mut(key.index)
            .filter(|slot| slot.generation == key.generation)
            .and_then(|slot| slot.value.as_mut())
            .ok_or(ReactiveError::Disposed)
    }
    fn remove(&mut self, key: Key) -> Option<T> {
        let slot = self.slots.get_mut(key.index)?;
        if slot.generation != key.generation {
            return None;
        }
        let value = slot.value.take()?;
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(key.index);
        }
        Some(value)
    }
    fn len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
    }
}
struct ScopeState {
    parent: Option<Key>,
    children: BTreeSet<Key>,
    signals: BTreeSet<Key>,
    effects: BTreeSet<Key>,
    resources: BTreeSet<Key>,
}
impl ScopeState {
    fn new(parent: Option<Key>) -> Self {
        Self {
            parent,
            children: BTreeSet::new(),
            signals: BTreeSet::new(),
            effects: BTreeSet::new(),
            resources: BTreeSet::new(),
        }
    }
}
struct ResourceState {
    owner: Key,
    value: Box<dyn Any>,
}
type Value = Rc<RefCell<Box<dyn Any>>>;
type Callback = Rc<RefCell<Box<dyn FnMut() -> Result<()>>>>;
struct SignalState {
    owner: Key,
    value: Value,
    subscribers: BTreeSet<Key>,
}
struct EffectState {
    owner: Key,
    callback: Callback,
    dependencies: BTreeSet<Key>,
    derived: bool,
    output: Option<Key>,
    queued: bool,
    running: bool,
    failure: Option<ReactiveError>,
}
#[derive(Default)]
struct Garbage {
    signals: Vec<SignalState>,
    effects: Vec<EffectState>,
    resources: Vec<Box<dyn Any>>,
}
struct Runtime {
    id: u64,
    active_owners: Vec<Key>,
    scopes: Arena<ScopeState>,
    signals: Arena<SignalState>,
    effects: Arena<EffectState>,
    resources: Arena<ResourceState>,
    owner: Key,
    current_effect: Option<Key>,
    tracking: bool,
    batch_depth: usize,
    processing: bool,
    effect_depth: usize,
    evaluations: usize,
    queue: VecDeque<Key>,
}
impl Runtime {
    fn new() -> Self {
        let mut scopes = Arena::default();
        let root = scopes.insert(ScopeState::new(None));
        let id = NEXT_RUNTIME
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .unwrap_or(0);
        Self {
            id,
            active_owners: Vec::new(),
            scopes,
            signals: Arena::default(),
            effects: Arena::default(),
            resources: Arena::default(),
            owner: root,
            current_effect: None,
            tracking: true,
            batch_depth: 0,
            processing: false,
            effect_depth: 0,
            evaluations: 0,
            queue: VecDeque::new(),
        }
    }
    fn validate_adoption(&self, moving: Owner, new_parent: Owner) -> Result<Option<Key>> {
        moving.check_runtime(self)?;
        new_parent.check_runtime(self)?;
        let old_parent = self
            .scopes
            .get(moving.key)?
            .parent
            .ok_or(ReactiveError::RootScope)?;
        self.scopes.get(new_parent.key)?;
        self.scopes.get(old_parent)?;
        if new_parent.key == old_parent {
            return Ok(None);
        }
        if self.scope_contains(moving.key, new_parent.key)? {
            return Err(ReactiveError::ScopeCycle);
        }
        for active in &self.active_owners {
            // A disposed context remains entered until its closure returns.
            if self.scopes.get(*active).is_ok() && self.scope_contains(moving.key, *active)? {
                return Err(ReactiveError::ActiveScope);
            }
        }
        Ok(Some(old_parent))
    }
    fn scope_contains(&self, ancestor: Key, descendant: Key) -> Result<bool> {
        let mut current = Some(descendant);
        for _ in 0..=self.scopes.slots.len() {
            let Some(key) = current else {
                return Ok(false);
            };
            if key == ancestor {
                return Ok(true);
            }
            current = self.scopes.get(key)?.parent;
        }
        Err(ReactiveError::ScopeCycle)
    }
    fn schedule(&mut self, key: Key) {
        let mut pending = vec![key];
        while let Some(key) = pending.pop() {
            let output = match self.effects.get_mut(key) {
                Ok(effect) if !effect.queued => {
                    effect.queued = true;
                    effect.failure = None;
                    self.queue.push_back(key);
                    effect.output
                }
                _ => continue,
            };
            if let Some(output) = output
                && let Ok(signal) = self.signals.get(output)
            {
                for dependent in signal.subscribers.iter().rev() {
                    if self
                        .effects
                        .get(*dependent)
                        .is_ok_and(|effect| effect.derived)
                    {
                        pending.push(*dependent);
                    }
                }
            }
        }
    }
    fn notify(&mut self, key: Key) -> Result<()> {
        let subscribers = self
            .signals
            .get(key)?
            .subscribers
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for effect in subscribers {
            self.schedule(effect);
        }
        Ok(())
    }
    fn track(&mut self, key: Key) -> Result<()> {
        if self.tracking
            && let Some(effect) = self.current_effect
        {
            self.effects.get_mut(effect)?.dependencies.insert(key);
            self.signals.get_mut(key)?.subscribers.insert(effect);
        }
        Ok(())
    }
    fn remove_effect(&mut self, key: Key, garbage: &mut Garbage) {
        if let Some(effect) = self.effects.remove(key) {
            for dep in &effect.dependencies {
                if let Ok(signal) = self.signals.get_mut(*dep) {
                    signal.subscribers.remove(&key);
                }
            }
            if let Ok(scope) = self.scopes.get_mut(effect.owner) {
                scope.effects.remove(&key);
            }
            self.queue.retain(|queued| *queued != key);
            garbage.effects.push(effect);
        }
    }
    fn remove_signal(&mut self, key: Key, garbage: &mut Garbage) {
        if let Some(signal) = self.signals.remove(key) {
            for effect in &signal.subscribers {
                if let Ok(effect) = self.effects.get_mut(*effect) {
                    effect.dependencies.remove(&key);
                }
            }
            if let Ok(scope) = self.scopes.get_mut(signal.owner) {
                scope.signals.remove(&key);
            }
            garbage.signals.push(signal);
        }
    }
    fn dispose_scope(&mut self, key: Key) -> Garbage {
        let mut garbage = Garbage::default();
        let mut stack = vec![(key, false)];
        while let Some((key, visited)) = stack.pop() {
            if !visited {
                if let Ok(scope) = self.scopes.get(key) {
                    stack.push((key, true));
                    stack.extend(
                        scope
                            .children
                            .iter()
                            .rev()
                            .copied()
                            .map(|child| (child, false)),
                    );
                }
            } else if let Some(scope) = self.scopes.remove(key) {
                for effect in scope.effects {
                    self.remove_effect(effect, &mut garbage);
                }
                for signal in scope.signals {
                    self.remove_signal(signal, &mut garbage);
                }
                for resource in scope.resources {
                    if let Some(resource) = self.resources.remove(resource) {
                        garbage.resources.push(resource.value);
                    }
                }
                if let Some(parent) = scope.parent
                    && let Ok(parent) = self.scopes.get_mut(parent)
                {
                    parent.children.remove(&key);
                }
            }
        }
        garbage
    }
    fn clear_queue(&mut self, reason: &ReactiveError) {
        for key in self.queue.drain(..) {
            if let Ok(effect) = self.effects.get_mut(key) {
                effect.queued = false;
                if effect.derived {
                    effect.failure = Some(reason.clone());
                }
            }
        }
    }
}
thread_local! { static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new()); }
fn runtime<R>(f: impl FnOnce(&mut Runtime) -> Result<R>) -> Result<R> {
    RUNTIME
        .try_with(|cell| {
            let mut state = cell
                .try_borrow_mut()
                .map_err(|_| ReactiveError::BorrowConflict)?;
            if state.id == 0 {
                return Err(ReactiveError::Exhausted);
            }
            f(&mut state)
        })
        .map_err(|_| ReactiveError::RuntimeUnavailable)?
}

/// A non-owning scope context that cannot cross threads.
#[derive(Clone, Copy, Debug)]
pub struct Owner {
    runtime: u64,
    key: Key,
    thread: PhantomData<Rc<()>>,
}
/// An owning scope. Dropping it disposes descendants, effects, and signals.
#[derive(Debug)]
pub struct Scope {
    owner: Owner,
}
impl Scope {
    /// Creates a child of the currently entered scope.
    pub fn new() -> Result<Self> {
        let (runtime_id, key) = runtime(|state| {
            state.scopes.get(state.owner)?;
            let key = state.scopes.insert(ScopeState::new(Some(state.owner)));
            state.scopes.get_mut(state.owner)?.children.insert(key);
            Ok((state.id, key))
        })?;
        Ok(Self {
            owner: Owner {
                runtime: runtime_id,
                key,
                thread: PhantomData,
            },
        })
    }
    /// Returns a copyable context without extending this scope's lifetime.
    pub fn owner(&self) -> Owner {
        self.owner
    }
    /// Runs construction under this scope's ownership.
    pub fn run<R>(&self, f: impl FnOnce() -> Result<R>) -> Result<R> {
        self.owner.run(f)
    }
    /// Checks whether this scope can change its lifetime parent without mutation.
    pub fn can_adopt(&self, new_parent: Owner) -> Result<()> {
        self.owner.can_adopt(new_parent)
    }
    /// Changes this scope's lifetime parent without recreating its resources.
    pub fn adopt(&self, new_parent: Owner) -> Result<()> {
        self.owner.adopt(new_parent)
    }
    /// Disposes this scope and descendants; repeated calls succeed.
    pub fn dispose(&self) -> Result<()> {
        let garbage = runtime(|state| {
            self.owner.check_runtime(state)?;
            Ok(state.dispose_scope(self.owner.key))
        })?;
        drop(garbage);
        Ok(())
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let _ = self.dispose();
    }
}
struct ContextGuard {
    entered_owner: bool,
    owner: Key,
    effect: Option<Key>,
    tracking: bool,
}
impl Drop for ContextGuard {
    fn drop(&mut self) {
        let _ = runtime(|state| {
            if self.entered_owner {
                state.active_owners.pop();
            }
            state.owner = self.owner;
            state.current_effect = self.effect;
            state.tracking = self.tracking;
            Ok(())
        });
    }
}
impl Owner {
    fn check_runtime(self, state: &Runtime) -> Result<()> {
        if self.runtime != state.id {
            Err(ReactiveError::ForeignRuntime)
        } else {
            Ok(())
        }
    }
    /// Checks adoption without changing scope ownership or executing callbacks.
    pub fn can_adopt(self, new_parent: Owner) -> Result<()> {
        runtime(|state| state.validate_adoption(self, new_parent).map(|_| ()))
    }
    /// Changes this scope's lifetime parent atomically, keeping identities and resources.
    ///
    /// Disposed and foreign owners, cycles, the implicit thread root, and moved
    /// subtrees containing an active callback context are rejected. The target
    /// parent may be active. Re-adopting the current parent is a successful no-op.
    /// External signal dependencies retain their original ownership.
    pub fn adopt(self, new_parent: Owner) -> Result<()> {
        runtime(|state| {
            let Some(old_parent) = state.validate_adoption(self, new_parent)? else {
                return Ok(());
            };
            let [old_slot, new_slot, moved_slot] = state
                .scopes
                .slots
                .get_disjoint_mut([old_parent.index, new_parent.key.index, self.key.index])
                .map_err(|_| ReactiveError::ScopeCycle)?;
            let (Some(old), Some(new), Some(moved)) = (
                old_slot.value.as_mut(),
                new_slot.value.as_mut(),
                moved_slot.value.as_mut(),
            ) else {
                return Err(ReactiveError::Disposed);
            };
            old.children.remove(&self.key);
            new.children.insert(self.key);
            moved.parent = Some(new_parent.key);
            Ok(())
        })
    }
    /// Runs construction in this context, rejecting disposed owners.
    pub fn run<R>(&self, f: impl FnOnce() -> Result<R>) -> Result<R> {
        let guard = runtime(|state| {
            self.check_runtime(state)?;
            state.scopes.get(self.key)?;
            let guard = ContextGuard {
                entered_owner: true,
                owner: state.owner,
                effect: state.current_effect,
                tracking: state.tracking,
            };
            state.active_owners.push(self.key);
            state.owner = self.key;
            Ok(guard)
        })?;
        let result = f();
        drop(guard);
        result
    }
}

/// A copyable, thread-local signal identity. Its scope owns the stored value.
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<oxiview_reactive::Signal<i32>>();
/// ```
pub struct Signal<T> {
    key: Key,
    thread: PhantomData<Rc<()>>,
    invariant: PhantomData<fn(T) -> T>,
}
impl<T> Copy for Signal<T> {}
impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> std::fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signal")
            .field("identity", &self.key)
            .finish()
    }
}
/// Creates a signal owned by the current scope.
pub fn signal<T: 'static>(value: T) -> Result<Signal<T>> {
    let value: Value = Rc::new(RefCell::new(Box::new(value)));
    let key = runtime(|state| {
        state.scopes.get(state.owner)?;
        let key = state.signals.insert(SignalState {
            owner: state.owner,
            value: Rc::clone(&value),
            subscribers: BTreeSet::new(),
        });
        state.scopes.get_mut(state.owner)?.signals.insert(key);
        Ok(key)
    })?;
    Ok(Signal {
        key,
        thread: PhantomData,
        invariant: PhantomData,
    })
}
impl<T: 'static> Signal<T> {
    fn value(&self) -> Result<Value> {
        runtime(|state| Ok(Rc::clone(&state.signals.get(self.key)?.value)))
    }
    /// Reads and tracks this signal while the value is immutably borrowed.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        let value = self.value()?;
        let borrowed = value
            .try_borrow()
            .map_err(|_| ReactiveError::BorrowConflict)?;
        let typed = borrowed
            .downcast_ref::<T>()
            .ok_or(ReactiveError::TypeMismatch)?;
        runtime(|state| state.track(self.key))?;
        Ok(f(typed))
    }
    /// Clones and tracks the current value.
    pub fn get(&self) -> Result<T>
    where
        T: Clone,
    {
        self.with(Clone::clone)
    }
    /// Replaces the value and notifies subscribers after releasing its borrow.
    pub fn set(&self, value: T) -> Result<()> {
        batch(|| {
            let cell = self.value()?;
            let old = {
                let mut borrowed = cell
                    .try_borrow_mut()
                    .map_err(|_| ReactiveError::BorrowConflict)?;
                if !borrowed.is::<T>() {
                    return Err(ReactiveError::TypeMismatch);
                }
                std::mem::replace(&mut *borrowed, Box::new(value))
            };
            runtime(|state| state.notify(self.key))?;
            drop(old);
            Ok(())
        })
    }
    /// Replaces a changed value, returning whether subscribers were notified.
    pub fn set_if_changed(&self, value: T) -> Result<bool>
    where
        T: PartialEq,
    {
        batch(|| {
            let cell = self.value()?;
            let old = {
                let mut borrowed = cell
                    .try_borrow_mut()
                    .map_err(|_| ReactiveError::BorrowConflict)?;
                let old = borrowed
                    .downcast_ref::<T>()
                    .ok_or(ReactiveError::TypeMismatch)?;
                if old == &value {
                    return Ok(false);
                }
                std::mem::replace(&mut *borrowed, Box::new(value))
            };
            runtime(|state| state.notify(self.key))?;
            drop(old);
            Ok(true)
        })
    }
    /// Mutates a value, batching nested writes and notifying after the borrow ends.
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R> {
        batch(|| {
            let cell = self.value()?;
            let result = {
                let mut borrowed = cell
                    .try_borrow_mut()
                    .map_err(|_| ReactiveError::BorrowConflict)?;
                let typed = borrowed
                    .downcast_mut::<T>()
                    .ok_or(ReactiveError::TypeMismatch)?;
                f(typed)
            };
            runtime(|state| state.notify(self.key))?;
            Ok(result)
        })
    }
    /// Releases this signal early without changing the ownership of other signals.
    pub fn dispose(&self) -> Result<()> {
        let garbage = runtime(|state| {
            state.signals.get(self.key)?;
            let mut garbage = Garbage::default();
            state.remove_signal(self.key, &mut garbage);
            Ok(garbage)
        })?;
        drop(garbage);
        Ok(())
    }
}

/// A non-owning effect identity; dropping this handle does not stop the effect.
#[derive(Clone, Copy, Debug)]
pub struct EffectHandle {
    key: Key,
    thread: PhantomData<Rc<()>>,
}
impl EffectHandle {
    /// Stops this effect and releases its subscriptions and callback.
    pub fn dispose(&self) -> Result<()> {
        let garbage = runtime(|state| {
            let mut garbage = Garbage::default();
            state.remove_effect(self.key, &mut garbage);
            Ok(garbage)
        })?;
        drop(garbage);
        Ok(())
    }
}
/// Creates a scope-owned observing effect and runs it after the current batch.
pub fn effect(f: impl FnMut() -> Result<()> + 'static) -> Result<EffectHandle> {
    create_effect(f, None)
}
fn create_effect(
    f: impl FnMut() -> Result<()> + 'static,
    output: Option<Key>,
) -> Result<EffectHandle> {
    let callback: Callback = Rc::new(RefCell::new(Box::new(f)));
    let key = runtime(|state| {
        state.scopes.get(state.owner)?;
        let key = state.effects.insert(EffectState {
            owner: state.owner,
            callback: Rc::clone(&callback),
            dependencies: BTreeSet::new(),
            derived: output.is_some(),
            output,
            queued: false,
            running: false,
            failure: None,
        });
        state.scopes.get_mut(state.owner)?.effects.insert(key);
        state.schedule(key);
        Ok(key)
    })?;
    let handle = EffectHandle {
        key,
        thread: PhantomData,
    };
    if let Err(error) = flush() {
        let _ = handle.dispose();
        return Err(error);
    }
    Ok(handle)
}
struct BatchGuard {
    active: bool,
}
impl BatchGuard {
    fn enter() -> Result<Self> {
        runtime(|state| {
            state.batch_depth = state
                .batch_depth
                .checked_add(1)
                .ok_or(ReactiveError::Exhausted)?;
            Ok(())
        })?;
        Ok(Self { active: true })
    }
    fn finish(mut self) -> Result<()> {
        runtime(|state| {
            state.batch_depth = state.batch_depth.saturating_sub(1);
            Ok(())
        })?;
        self.active = false;
        flush()
    }
}
impl Drop for BatchGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = runtime(|state| {
                state.batch_depth = state.batch_depth.saturating_sub(1);
                Ok(())
            });
        }
    }
}
/// Groups writes so observers see the final values and run at most once if settled.
pub fn batch<R>(f: impl FnOnce() -> Result<R>) -> Result<R> {
    let guard = BatchGuard::enter()?;
    let result = f();
    let settled = guard.finish();
    match result {
        Ok(value) => {
            settled?;
            Ok(value)
        }
        Err(error) => Err(error),
    }
}
/// Reads without subscribing the currently executing effect.
pub fn untrack<R>(f: impl FnOnce() -> Result<R>) -> Result<R> {
    let guard = runtime(|state| {
        let guard = ContextGuard {
            entered_owner: false,
            owner: state.owner,
            effect: state.current_effect,
            tracking: state.tracking,
        };
        state.tracking = false;
        Ok(guard)
    })?;
    let result = f();
    drop(guard);
    result
}
struct ProcessingGuard;
impl Drop for ProcessingGuard {
    fn drop(&mut self) {
        let _ = runtime(|state| {
            state.processing = false;
            Ok(())
        });
    }
}
struct EffectGuard {
    _context: ContextGuard,
    key: Key,
}
impl Drop for EffectGuard {
    fn drop(&mut self) {
        let _ = runtime(|state| {
            state.effect_depth = state.effect_depth.saturating_sub(1);
            if let Ok(effect) = state.effects.get_mut(self.key) {
                effect.running = false;
            }
            Ok(())
        });
    }
}
fn execute(key: Key) -> Result<()> {
    let (callback, guard) = runtime(|state| {
        let effect = state.effects.get(key)?;
        if effect.running {
            return Err(ReactiveError::Cycle);
        }
        if !state.processing && state.effect_depth == 0 {
            state.evaluations = 0;
        }
        if state.effect_depth >= MAX_EFFECT_DEPTH || state.evaluations >= EFFECT_BUDGET {
            let effect = state.effects.get_mut(key)?;
            effect.queued = false;
            effect.failure = Some(ReactiveError::UpdateLimit);
            state.queue.retain(|queued| *queued != key);
            return Err(ReactiveError::UpdateLimit);
        }
        let effect = state.effects.get(key)?;
        let owner = effect.owner;
        let callback = Rc::clone(&effect.callback);
        let deps = effect.dependencies.clone();
        state.scopes.get(owner)?;
        for dep in deps {
            if let Ok(signal) = state.signals.get_mut(dep) {
                signal.subscribers.remove(&key);
            }
        }
        let effect = state.effects.get_mut(key)?;
        effect.dependencies.clear();
        effect.running = true;
        effect.queued = false;
        state.queue.retain(|queued| *queued != key);
        let context = ContextGuard {
            entered_owner: true,
            owner: state.owner,
            effect: state.current_effect,
            tracking: state.tracking,
        };
        state.active_owners.push(owner);
        state.owner = owner;
        state.current_effect = Some(key);
        state.tracking = true;
        state.effect_depth += 1;
        state.evaluations += 1;
        Ok((
            callback,
            EffectGuard {
                _context: context,
                key,
            },
        ))
    })?;
    let result = callback
        .try_borrow_mut()
        .map_err(|_| ReactiveError::BorrowConflict)?
        .as_mut()();
    drop(guard);
    runtime(|state| {
        if let Ok(effect) = state.effects.get_mut(key) {
            effect.failure = result.as_ref().err().cloned();
        }
        Ok(())
    })?;
    result
}
/// Settles pending derived values and effects without polling or timers.
pub fn flush() -> Result<()> {
    let should_run = runtime(|state| {
        if state.processing || state.batch_depth > 0 || state.effect_depth > 0 {
            return Ok(false);
        }
        state.processing = true;
        state.evaluations = 0;
        Ok(true)
    })?;
    if !should_run {
        return Ok(());
    }
    let _guard = ProcessingGuard;
    for _ in 0..EFFECT_BUDGET {
        let next = runtime(|state| {
            state.queue.retain(|key| state.effects.get(*key).is_ok());
            let position = state
                .queue
                .iter()
                .position(|key| state.effects.get(*key).is_ok_and(|effect| effect.derived));
            Ok(if let Some(position) = position {
                state.queue.remove(position)
            } else {
                state.queue.pop_front()
            })
        })?;
        let Some(key) = next else {
            return Ok(());
        };
        if let Err(error) = execute(key) {
            runtime(|state| {
                state.clear_queue(&error);
                Ok(())
            })?;
            return Err(error);
        }
    }
    let pending = runtime(|state| {
        let pending = !state.queue.is_empty();
        state.clear_queue(&ReactiveError::UpdateLimit);
        Ok(pending)
    })?;
    if pending {
        Err(ReactiveError::UpdateLimit)
    } else {
        Ok(())
    }
}

/// A lazily readable derived value whose updates settle before observing effects.
pub struct Memo<T> {
    value: Signal<Option<T>>,
    effect: EffectHandle,
}
impl<T> Copy for Memo<T> {}
impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> std::fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memo")
            .field("effect", &self.effect)
            .finish()
    }
}
/// Creates a cached derived computation with equality-based change suppression.
pub fn memo<T: Clone + PartialEq + 'static>(
    mut compute: impl FnMut() -> Result<T> + 'static,
) -> Result<Memo<T>> {
    let value = signal(None)?;
    let effect = match create_effect(
        move || {
            value.set_if_changed(Some(compute()?))?;
            Ok(())
        },
        Some(value.key),
    ) {
        Ok(effect) => effect,
        Err(error) => {
            let _ = value.dispose();
            return Err(error);
        }
    };
    Ok(Memo { value, effect })
}
impl<T: Clone + PartialEq + 'static> Memo<T> {
    /// Reads the current derived value, recomputing dirty dependencies on demand.
    pub fn get(&self) -> Result<T> {
        let (queued, running, failure) = runtime(|state| {
            let effect = state.effects.get(self.effect.key)?;
            Ok((effect.queued, effect.running, effect.failure.clone()))
        })?;
        if running {
            return Err(ReactiveError::Cycle);
        }
        if let Some(error) = failure {
            runtime(|state| state.track(self.value.key))?;
            return Err(error);
        }
        if queued && let Err(error) = execute(self.effect.key) {
            runtime(|state| state.track(self.value.key))?;
            return Err(error);
        }
        self.value.get()?.ok_or(ReactiveError::Uninitialized)
    }
}

/// Current live resource counts for deterministic lifecycle diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeStats {
    /// Live scopes, including the thread root.
    pub scopes: usize,
    /// Live signal storage slots.
    pub signals: usize,
    /// Live effects and derived computations.
    pub effects: usize,
    /// Scope-owned auxiliary resources, including local-task cancellation guards.
    pub resources: usize,
    /// Pending effect evaluations.
    pub pending: usize,
}
/// Inspects live runtime resource counts without creating subscriptions.
pub fn runtime_stats() -> Result<RuntimeStats> {
    runtime(|state| {
        Ok(RuntimeStats {
            scopes: state.scopes.len(),
            signals: state.signals.len(),
            effects: state.effects.len(),
            resources: state.resources.len(),
            pending: state.queue.len(),
        })
    })
}

#[cfg(test)]
mod tests;

fn current_owner() -> Result<Owner> {
    runtime(|state| {
        state.scopes.get(state.owner)?;
        Ok(Owner {
            runtime: state.id,
            key: state.owner,
            thread: PhantomData,
        })
    })
}
fn hold_resource(owner: Owner, value: Box<dyn Any>) -> Result<Key> {
    let mut value = Some(value);
    runtime(|state| {
        owner.check_runtime(state)?;
        state.scopes.get(owner.key)?;
        let value = value.take().ok_or(ReactiveError::Disposed)?;
        let key = state.resources.insert(ResourceState {
            owner: owner.key,
            value,
        });
        state.scopes.get_mut(owner.key)?.resources.insert(key);
        Ok(key)
    })
}
fn release_resource(key: Key) -> Result<()> {
    let value = runtime(|state| {
        let Some(resource) = state.resources.remove(key) else {
            return Ok(None);
        };
        if let Ok(scope) = state.scopes.get_mut(resource.owner) {
            scope.resources.remove(&key);
        }
        Ok(Some(resource.value))
    })?;
    drop(value);
    Ok(())
}
