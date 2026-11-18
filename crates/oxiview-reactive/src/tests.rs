use super::*;
use std::cell::Cell;

#[test]
fn copied_handles_reject_disposal_and_reuse() -> Result<()> {
    let scope = Scope::new()?;
    let old = scope.run(|| signal(String::from("old")))?;
    let copy = old;
    scope.dispose()?;
    let replacement_scope = Scope::new()?;
    let fresh = replacement_scope.run(|| signal(String::from("fresh")))?;
    assert!(matches!(old.get(), Err(ReactiveError::Disposed)));
    assert!(matches!(
        copy.set(String::new()),
        Err(ReactiveError::Disposed)
    ));
    assert_eq!(fresh.get()?, "fresh");
    Ok(())
}

#[test]
fn owner_handles_do_not_keep_parent_scopes_alive() -> Result<()> {
    let before = runtime_stats()?;
    let parent = Scope::new()?;
    let owner = parent.owner();
    let (child, value) = parent.run(|| {
        let child = Scope::new()?;
        let value = child.run(|| signal(7))?;
        Ok((child, value))
    })?;
    drop(parent);
    assert!(matches!(value.get(), Err(ReactiveError::Disposed)));
    assert!(matches!(
        owner.run(|| signal(1)),
        Err(ReactiveError::Disposed)
    ));
    child.dispose()?;
    assert_eq!(runtime_stats()?, before);
    Ok(())
}

#[test]
fn effects_retrack_dynamic_dependencies() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let left = signal(1)?;
        let right = signal(10)?;
        let choose_left = signal(true)?;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let output = Rc::clone(&seen);
        let handle = effect(move || {
            output.borrow_mut().push(if choose_left.get()? {
                left.get()?
            } else {
                right.get()?
            });
            Ok(())
        })?;
        left.set(2)?;
        choose_left.set(false)?;
        left.set(3)?;
        right.set(11)?;
        assert_eq!(*seen.borrow(), vec![1, 2, 10, 11]);
        handle.dispose()?;
        right.set(12)?;
        assert_eq!(*seen.borrow(), vec![1, 2, 10, 11]);
        Ok(())
    })
}

#[test]
fn derived_diamonds_settle_before_observers_in_a_batch() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let source = signal(1)?;
        let doubled = memo(move || Ok(source.get()? * 2))?;
        let tripled = memo(move || Ok(source.get()? * 3))?;
        let sum = memo(move || Ok(doubled.get()? + tripled.get()?))?;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let output = Rc::clone(&seen);
        effect(move || {
            output.borrow_mut().push((source.get()?, sum.get()?));
            Ok(())
        })?;
        batch(|| {
            source.set(2)?;
            assert_eq!(sum.get()?, 10);
            source.set(3)?;
            Ok(())
        })?;
        assert_eq!(*seen.borrow(), vec![(1, 5), (3, 15)]);
        Ok(())
    })
}

#[test]
fn unchanged_memos_do_not_notify_observers() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let source = signal(1)?;
        let parity = memo(move || Ok(source.get()? % 2))?;
        let calls = Rc::new(Cell::new(0));
        let count = Rc::clone(&calls);
        effect(move || {
            let _ = parity.get()?;
            count.set(count.get() + 1);
            Ok(())
        })?;
        source.set(3)?;
        assert_eq!(calls.get(), 1);
        source.set(4)?;
        assert_eq!(calls.get(), 2);
        Ok(())
    })
}

#[test]
fn update_batches_nested_writes_and_rejects_conflicting_borrows() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let a = signal(1)?;
        let b = signal(1)?;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let output = Rc::clone(&seen);
        effect(move || {
            output.borrow_mut().push(a.get()? + b.get()?);
            Ok(())
        })?;
        a.update(|value| {
            *value = 2;
            b.set(3)
        })??;
        assert_eq!(*seen.borrow(), vec![2, 5]);
        let nested = a.update(|_| a.get())?;
        assert!(matches!(nested, Err(ReactiveError::BorrowConflict)));
        a.set(4)?;
        assert_eq!(a.get()?, 4);
        Ok(())
    })
}

#[test]
fn disposed_pending_effects_do_not_run_and_resources_drop_once() -> Result<()> {
    struct CountDrop(Rc<Cell<usize>>);
    impl Drop for CountDrop {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let scope = Scope::new()?;
    let dropped = Rc::new(Cell::new(0));
    let calls = Rc::new(Cell::new(0));
    let value = scope.run(|| {
        let value = signal(0)?;
        let resource = CountDrop(Rc::clone(&dropped));
        let count = Rc::clone(&calls);
        effect(move || {
            let _ = &resource;
            let _ = value.get()?;
            count.set(count.get() + 1);
            Ok(())
        })?;
        Ok(value)
    })?;
    batch(|| {
        value.set(1)?;
        scope.dispose()?;
        Ok(())
    })?;
    assert_eq!(calls.get(), 1);
    assert_eq!(dropped.get(), 1);
    scope.dispose()?;
    assert_eq!(dropped.get(), 1);
    assert_eq!(runtime_stats()?.pending, 0);
    Ok(())
}

#[test]
fn untracked_reads_do_not_add_dependencies() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let tracked = signal(1)?;
        let ignored = signal(2)?;
        let calls = Rc::new(Cell::new(0));
        let count = Rc::clone(&calls);
        effect(move || {
            let _ = tracked.get()?;
            let _ = untrack(|| ignored.get())?;
            count.set(count.get() + 1);
            Ok(())
        })?;
        ignored.set(3)?;
        assert_eq!(calls.get(), 1);
        tracked.set(2)?;
        assert_eq!(calls.get(), 2);
        Ok(())
    })
}

#[test]
fn callback_errors_leave_the_runtime_reusable() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let value = signal(0)?;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let output = Rc::clone(&seen);
        effect(move || {
            let value = value.get()?;
            output.borrow_mut().push(value);
            if value == 2 {
                Err(ReactiveError::User("two".into()))
            } else {
                Ok(())
            }
        })?;
        assert!(matches!(value.set(2), Err(ReactiveError::User(_))));
        value.set(3)?;
        assert_eq!(*seen.borrow(), vec![0, 2, 3]);
        Ok(())
    })
}

#[test]
fn self_triggering_effects_are_bounded_and_initial_failure_is_disposed() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let value = signal(0usize)?;
        let before = runtime_stats()?.effects;
        let result = effect(move || value.set(value.get()? + 1));
        assert!(matches!(result, Err(ReactiveError::UpdateLimit)));
        assert_eq!(runtime_stats()?.effects, before);
        assert_eq!(runtime_stats()?.pending, 0);
        value.set(0)?;
        assert_eq!(value.get()?, 0);
        Ok(())
    })
}

#[test]
fn failed_derived_values_are_not_returned_as_fresh_cache_entries() -> Result<()> {
    let scope = Scope::new()?;
    scope.run(|| {
        let source = signal(1)?;
        let doubled = memo(move || {
            let value = source.get()?;
            if value == 2 {
                Err(ReactiveError::User("invalid two".into()))
            } else {
                Ok(value * 2)
            }
        })?;
        let descendant = memo(move || Ok(doubled.get()? + 1))?;
        assert_eq!(descendant.get()?, 3);
        assert!(matches!(source.set(2), Err(ReactiveError::User(_))));
        assert!(matches!(doubled.get(), Err(ReactiveError::User(_))));
        assert!(matches!(descendant.get(), Err(ReactiveError::User(_))));
        source.set(3)?;
        assert_eq!(descendant.get()?, 7);
        Ok(())
    })
}

#[test]
fn adopted_subtree_keeps_private_resources_but_not_external_ancestor_signals() -> Result<()> {
    struct CountDrop(Rc<Cell<usize>>);
    impl Drop for CountDrop {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let before = runtime_stats()?;
    let old_parent = Scope::new()?;
    let new_parent = Scope::new()?;
    let external = old_parent.run(|| signal(100))?;
    let moving = old_parent.run(Scope::new)?;
    let signal_drops = Rc::new(Cell::new(0));
    let effect_drops = Rc::new(Cell::new(0));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let private = moving.run(|| {
        let value = signal(1)?;
        let _resource = signal(CountDrop(Rc::clone(&signal_drops)))?;
        let resource = CountDrop(Rc::clone(&effect_drops));
        let output = Rc::clone(&calls);
        effect(move || {
            let _ = &resource;
            output.borrow_mut().push(value.get()?);
            Ok(())
        })?;
        Ok(value)
    })?;
    let descendant = moving.run(Scope::new)?;
    let child = descendant.run(|| signal(9))?;
    new_parent.run(|| moving.adopt(new_parent.owner()))?;
    assert_eq!(*calls.borrow(), vec![1]);
    old_parent.dispose()?;
    assert!(matches!(external.get(), Err(ReactiveError::Disposed)));
    assert_eq!(private.get()?, 1);
    assert_eq!(child.get()?, 9);
    private.set(2)?;
    assert_eq!(*calls.borrow(), vec![1, 2]);
    assert_eq!(signal_drops.get(), 0);
    assert_eq!(effect_drops.get(), 0);
    new_parent.dispose()?;
    assert!(matches!(private.get(), Err(ReactiveError::Disposed)));
    assert!(matches!(child.get(), Err(ReactiveError::Disposed)));
    assert_eq!(signal_drops.get(), 1);
    assert_eq!(effect_drops.get(), 1);
    moving.dispose()?;
    descendant.dispose()?;
    assert_eq!(signal_drops.get(), 1);
    assert_eq!(effect_drops.get(), 1);
    assert_eq!(runtime_stats()?, before);
    Ok(())
}

#[test]
fn adoption_validation_rejects_cycles_stale_owners_and_root_without_detaching() -> Result<()> {
    let old = Scope::new()?;
    let target = Scope::new()?;
    let dead = Scope::new()?;
    let moving = old.run(Scope::new)?;
    let descendant = moving.run(Scope::new)?;
    let private = moving.run(|| signal(7))?;
    dead.dispose()?;
    assert!(matches!(
        moving.can_adopt(moving.owner()),
        Err(ReactiveError::ScopeCycle)
    ));
    assert!(matches!(
        moving.adopt(descendant.owner()),
        Err(ReactiveError::ScopeCycle)
    ));
    assert!(matches!(
        moving.adopt(dead.owner()),
        Err(ReactiveError::Disposed)
    ));
    assert!(matches!(
        dead.adopt(target.owner()),
        Err(ReactiveError::Disposed)
    ));
    assert!(matches!(
        current_owner()?.adopt(target.owner()),
        Err(ReactiveError::RootScope)
    ));
    moving.can_adopt(target.owner())?;
    // Successful validation alone must not move ownership.
    old.dispose()?;
    assert!(matches!(private.get(), Err(ReactiveError::Disposed)));
    assert!(matches!(
        descendant.run(|| signal(1)),
        Err(ReactiveError::Disposed)
    ));
    assert_eq!(target.run(|| signal(2))?.get()?, 2);
    Ok(())
}

#[test]
fn active_nested_contexts_reject_adoption_atomically_and_noops_are_allowed() -> Result<()> {
    let old = Scope::new()?;
    let target = Scope::new()?;
    let unrelated = Scope::new()?;
    let moving = old.run(Scope::new)?;
    let descendant = moving.run(Scope::new)?;
    let private = moving.run(|| signal(3))?;
    moving.run(|| {
        moving.adopt(old.owner())?;
        unrelated.run(|| {
            assert!(matches!(
                moving.can_adopt(target.owner()),
                Err(ReactiveError::ActiveScope)
            ));
            assert!(matches!(
                moving.adopt(target.owner()),
                Err(ReactiveError::ActiveScope)
            ));
            Ok(())
        })
    })?;
    descendant.run(|| {
        assert!(matches!(
            moving.adopt(target.owner()),
            Err(ReactiveError::ActiveScope)
        ));
        Ok(())
    })?;
    old.dispose()?;
    assert!(matches!(private.get(), Err(ReactiveError::Disposed)));

    let other = Scope::new()?;
    let moved = other.run(Scope::new)?;
    let value = moved.run(|| signal(4))?;
    moved.run(|| {
        assert!(matches!(
            moved.adopt(target.owner()),
            Err(ReactiveError::ActiveScope)
        ));
        Ok(())
    })?;
    moved.adopt(target.owner())?;
    other.dispose()?;
    assert_eq!(value.get()?, 4);
    target.dispose()?;
    assert!(matches!(value.get(), Err(ReactiveError::Disposed)));
    Ok(())
}

#[test]
fn adoption_checks_running_effects_and_preserves_the_rejected_owner() -> Result<()> {
    let old = Scope::new()?;
    let target = Scope::new()?;
    let moving = old.run(Scope::new)?;
    let trigger = moving.run(|| signal(0))?;
    let owner = moving.owner();
    let destination = target.owner();
    let observed = Rc::new(Cell::new(false));
    let result = Rc::clone(&observed);
    moving.run(|| {
        effect(move || {
            let _ = trigger.get()?;
            result.set(matches!(
                owner.adopt(destination),
                Err(ReactiveError::ActiveScope)
            ));
            Ok(())
        })?;
        Ok(())
    })?;
    assert!(observed.get());
    old.dispose()?;
    assert!(matches!(trigger.get(), Err(ReactiveError::Disposed)));
    Ok(())
}

#[test]
fn foreign_runtime_owners_cannot_alias_local_scope_slots() -> Result<()> {
    let moving = Scope::new()?;
    let target = Scope::new()?;
    let value = moving.run(|| signal(8))?;
    let foreign_runtime = std::thread::spawn(|| runtime(|state| Ok(state.id)))
        .join()
        .map_err(|_| ReactiveError::User("foreign runtime test thread failed".into()))??;
    let foreign = Owner {
        runtime: foreign_runtime,
        key: target.owner().key,
        thread: PhantomData,
    };
    assert!(matches!(
        moving.adopt(foreign),
        Err(ReactiveError::ForeignRuntime)
    ));
    assert!(matches!(
        foreign.adopt(target.owner()),
        Err(ReactiveError::ForeignRuntime)
    ));
    assert!(matches!(
        foreign.run(|| signal(5)),
        Err(ReactiveError::ForeignRuntime)
    ));
    assert_eq!(value.get()?, 8);
    moving.dispose()?;
    assert!(matches!(value.get(), Err(ReactiveError::Disposed)));
    Ok(())
}

#[test]
fn scope_owned_local_tasks_follow_adoption_and_new_parent_cleanup() -> Result<()> {
    struct Resource(Rc<Cell<usize>>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let executor = LocalExecutor::new(|| {});
    let old = Scope::new()?;
    let new = Scope::new()?;
    let moving = old.run(Scope::new)?;
    let dropped = Rc::new(Cell::new(0));
    let resource = Resource(Rc::clone(&dropped));
    let task = moving.run(|| {
        executor.spawn_local(async move {
            std::future::pending::<()>().await;
            drop(resource);
            Ok(())
        })
    })?;
    executor.poll_ready(8)?;
    moving.adopt(new.owner())?;
    old.dispose()?;
    assert!(!task.is_finished()?);
    assert_eq!(dropped.get(), 0);
    new.dispose()?;
    assert!(task.is_finished()?);
    assert_eq!(dropped.get(), 1);
    assert_eq!(runtime_stats()?.resources, 0);
    Ok(())
}

#[test]
fn queued_effects_follow_the_scope_when_adopted_before_batch_completion() -> Result<()> {
    let old = Scope::new()?;
    let new = Scope::new()?;
    let moving = old.run(Scope::new)?;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let output = Rc::clone(&seen);
    let private = moving.run(|| {
        let private = signal(1)?;
        effect(move || {
            output.borrow_mut().push(private.get()?);
            Ok(())
        })?;
        Ok(private)
    })?;
    batch(|| {
        private.set(2)?;
        moving.adopt(new.owner())?;
        old.dispose()?;
        Ok(())
    })?;
    assert_eq!(*seen.borrow(), vec![1, 2]);
    new.dispose()?;
    assert!(matches!(private.get(), Err(ReactiveError::Disposed)));
    Ok(())
}

#[test]
fn an_active_local_task_cannot_move_its_scope_owner() -> Result<()> {
    let executor = LocalExecutor::new(|| {});
    let old = Scope::new()?;
    let target = Scope::new()?;
    let moving = old.run(Scope::new)?;
    let owner = moving.owner();
    let destination = target.owner();
    let private = moving.run(|| signal(11))?;
    moving.run(|| {
        executor.spawn_local(async move {
            assert!(matches!(
                owner.can_adopt(destination),
                Err(ReactiveError::ActiveScope)
            ));
            assert!(matches!(
                owner.adopt(destination),
                Err(ReactiveError::ActiveScope)
            ));
            Ok(())
        })
    })?;
    let report = executor.poll_ready(8)?;
    assert_eq!(report.completed, 1);
    assert!(report.errors.is_empty());
    old.dispose()?;
    assert!(matches!(private.get(), Err(ReactiveError::Disposed)));
    Ok(())
}
