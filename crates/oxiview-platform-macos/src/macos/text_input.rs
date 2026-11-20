//! The AppKit input-context client. Each focus lifetime gets a distinct
//! Objective-C client so callbacks retained by an IME cannot target a new field.

use super::{
    Native, Shared, boundary, find, native_time,
    text_state::{MISSING, TextState, byte_range, first_rect, index_at_point, native_range},
};
use objc2::{
    AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject, Sel},
};
use objc2_app_kit::{NSEvent, NSTextInputClient, NSTextInputContext};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSObject, NSObjectProtocol, NSPoint,
    NSRange, NSRangePointer, NSRect, NSSize, NSString,
};
use oxiview_base::{Point, Rect};
use oxiview_platform::{
    KeyCode, KeyModifiers, TextEditingCommand, TextInputEdit, TextInputEvent, TextInputSession,
    TextInputSnapshot, WindowEvent,
};
use std::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    sync::Arc,
    time::Duration,
};

struct KeyTurn {
    code: KeyCode,
    modifiers: KeyModifiers,
    timestamp: Duration,
    edits: Vec<TextInputEvent>,
    claimed: bool,
    unclaimed: bool,
    discard_mark: bool,
}

struct ClientState {
    shared: Arc<Shared>,
    text: RefCell<TextState>,
    key_turn: RefCell<Option<KeyTurn>>,
    retired: Cell<bool>,
    discarding: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject supports subclassing. AppKit invokes this client on the
    // main thread, its declared thread affinity.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ClientState]
    struct TextClient;

    // SAFETY: NSObjectProtocol has no additional invariants.
    unsafe impl NSObjectProtocol for TextClient {}

    // SAFETY: The selectors below use the NSTextInputClient signature. The
    // client owns its snapshot, and converts external pointers and ranges
    // within the call. AppKit obtains it from a main-thread input context.
    unsafe impl NSTextInputClient for TextClient {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, replacement: NSRange) {
            boundary(|| {
                if !self.window_is_key() {
                    return;
                }
                let text = input_string(string);
                let unclaimed = !self.has_mark()
                    && self.ivars().key_turn.borrow().as_ref().is_some_and(|turn| {
                        matches!(
                            (turn.code, text.as_deref()),
                            (KeyCode::Enter, Some("\r" | "\n")) | (KeyCode::Tab, Some("\t"))
                        )
                    });
                if unclaimed {
                    self.mark_unclaimed();
                } else {
                    self.callback(move |state, time| state.commit(text?, replacement, time));
                }
            });
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked(&self, string: &AnyObject, selected: NSRange, replacement: NSRange) {
            boundary(|| {
                if !self.window_is_key() {
                    return;
                }
                let text = input_string(string);
                self.callback(move |state, time| state.mark(text?, selected, replacement, time));
            });
        }

        #[unsafe(method(unmarkText))]
        fn unmark(&self) {
            boundary(|| {
                if !self.has_mark() {
                    return;
                }
                if self.escape_turn() || !self.window_is_key() {
                    self.cancel_mark();
                } else {
                    self.callback(TextState::unmark);
                }
            });
        }

        #[unsafe(method(doCommandBySelector:))]
        fn command(&self, selector: Sel) {
            boundary(|| self.command_by_selector(selector));
        }

        #[unsafe(method(selectedRange))]
        fn selected(&self) -> NSRange {
            boundary(|| {
                self.query(|snapshot| native_range(&snapshot.text, snapshot.selection.clone()))
                    .flatten()
                    .unwrap_or(MISSING)
            })
        }

        #[unsafe(method(markedRange))]
        fn marked(&self) -> NSRange {
            boundary(|| {
                self.query(|snapshot| native_range(&snapshot.text, snapshot.marked.clone()?))
                    .flatten()
                    .unwrap_or(MISSING)
            })
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked(&self) -> bool {
            boundary(|| self.has_mark())
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            boundary(NSArray::new)
        }

        /// # Safety
        /// AppKit supplies a writable NSRange out-parameter or a null pointer.
        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn substring(
            &self,
            range: NSRange,
            actual: NSRangePointer,
        ) -> Option<Retained<NSAttributedString>> {
            boundary(|| {
                let substring = self
                    .query(|snapshot| {
                        let bytes = byte_range(&snapshot.text, range, true)?;
                        let actual = native_range(&snapshot.text, bytes.clone())?;
                        let text = NSString::from_str(snapshot.text.get(bytes)?);
                        Some((
                            actual,
                            NSAttributedString::initWithString(NSAttributedString::alloc(), &text),
                        ))
                    })
                    .flatten();
                if let Some(actual) = NonNull::new(actual) {
                    // SAFETY: NSTextInputClient supplies a nullable, writable out-parameter
                    // for this synchronous callback. We retain no pointer to the result.
                    unsafe {
                        actual
                            .as_ptr()
                            .write(substring.as_ref().map_or(MISSING, |(range, _)| *range));
                    }
                }
                substring.map(|(_, text)| text)
            })
        }

        /// # Safety
        /// AppKit supplies a writable NSRange out-parameter or a null pointer.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn first_character_rect(&self, range: NSRange, actual: NSRangePointer) -> NSRect {
            boundary(|| {
                let result = self
                    .query(|snapshot| {
                        let bytes = byte_range(&snapshot.text, range, true)?;
                        let (actual, rect) = first_rect(snapshot, bytes);
                        Some((native_range(&snapshot.text, actual)?, rect))
                    })
                    .flatten();
                let result = result.and_then(|(actual, rect)| {
                    let window = find(self.ivars().shared.id)?;
                    Some((actual, screen_rect(&window, rect)))
                });
                if let Some(actual) = NonNull::new(actual) {
                    // SAFETY: NSTextInputClient supplies a nullable, writable out-parameter
                    // for this synchronous callback. We retain no pointer to the result.
                    unsafe {
                        actual
                            .as_ptr()
                            .write(result.map_or(MISSING, |(range, _)| range));
                    }
                }
                result.map_or(NSRect::ZERO, |(_, rect)| rect)
            })
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, point: NSPoint) -> usize {
            boundary(|| {
                let Some(window) = find(self.ivars().shared.id) else {
                    return MISSING.location;
                };
                let point = window
                    .view
                    .convertPoint_fromView(window.window.convertPointFromScreen(point), None);
                self.query(|snapshot| {
                    let index = index_at_point(snapshot, Point::new(point.x, point.y))?;
                    Some(native_range(&snapshot.text, index..index)?.location)
                })
                .flatten()
                .unwrap_or(MISSING.location)
            })
        }

        #[unsafe(method(windowLevel))]
        fn level(&self) -> isize {
            boundary(|| find(self.ivars().shared.id).map_or(0, |window| window.window.level()))
        }

        #[unsafe(method(drawsVerticallyForCharacterAtIndex:))]
        fn draws_vertically(&self, _index: usize) -> bool {
            false
        }
    }
);

impl TextClient {
    fn window_is_key(&self) -> bool {
        !self.ivars().retired.get()
            && find(self.ivars().shared.id).is_some_and(|window| window.window.isKeyWindow())
    }

    fn query<T>(&self, query: impl FnOnce(&TextInputSnapshot) -> T) -> Option<T> {
        (!self.ivars().retired.get()).then(|| query(self.ivars().text.borrow().snapshot()))
    }

    fn has_mark(&self) -> bool {
        self.query(|snapshot| snapshot.marked.is_some())
            .unwrap_or_default()
    }

    fn escape_turn(&self) -> bool {
        self.ivars()
            .key_turn
            .borrow()
            .as_ref()
            .is_some_and(|turn| turn.code == KeyCode::Escape)
    }

    fn callback(&self, apply: impl FnOnce(&mut TextState, Duration) -> Option<TextInputEvent>) {
        let state = self.ivars();
        if state.retired.get() || state.discarding.get() {
            return;
        }
        let mut turn = state.key_turn.borrow_mut();
        let timestamp = turn
            .as_ref()
            .map_or_else(native_time, |turn| turn.timestamp);
        let event = apply(&mut state.text.borrow_mut(), timestamp);
        if let Some(turn) = turn.as_mut() {
            // Invalid native ranges still consume the calling stroke.
            turn.claimed = true;
            turn.edits.extend(event);
        } else if let Some(event) = event
            && let Some(window) = find(state.shared.id)
        {
            window.route_input(WindowEvent::TextInput(event));
        }
    }

    fn cancel_mark(&self) {
        self.callback(|state, time| state.edit(TextInputEdit::Cancel, time));
    }

    fn mark_unclaimed(&self) {
        if let Some(turn) = self.ivars().key_turn.borrow_mut().as_mut() {
            turn.unclaimed = true;
        }
    }

    fn command_by_selector(&self, selector: Sel) {
        if !self.window_is_key() {
            return;
        }
        let name = selector.name().to_bytes();
        if name == b"cancelOperation:" && self.has_mark() {
            self.cancel_mark();
            if let Some(turn) = self.ivars().key_turn.borrow_mut().as_mut() {
                turn.discard_mark = true;
            }
            return;
        }
        let raw = self.ivars().key_turn.borrow().as_ref().is_some_and(|turn| {
            use KeyCode::*;
            matches!(
                turn.code,
                Tab | Escape
                    | Enter
                    | ArrowLeft
                    | ArrowRight
                    | ArrowUp
                    | ArrowDown
                    | Backspace
                    | Delete
                    | Home
                    | End
                    | PageUp
                    | PageDown
            ) && !turn.modifiers.control
        });
        if !raw && let Some(command) = editing_command(name) {
            self.callback(|state, time| state.edit(TextInputEdit::Command(command), time));
        } else {
            self.mark_unclaimed();
        }
    }
}

fn screen_rect(window: &Native, rect: Rect) -> NSRect {
    let [x0, y0, x1, y1] = rect.edges();
    let view_rect = NSRect::new(NSPoint::new(x0, y0), NSSize::new(x1 - x0, y1 - y0));
    window
        .window
        .convertRectToScreen(window.view.convertRect_toView(view_rect, None))
}

pub(super) fn owned_string(string: &NSString) -> Option<String> {
    let len = string.length();
    if len == 0 {
        return Some(String::new());
    }
    let mut units = vec![0; len];
    let buffer = NonNull::new(units.as_mut_ptr())?;
    // SAFETY: The vector has exactly len initialized UTF-16 slots, and the
    // immutable NSString range begins at zero and covers its reported length.
    unsafe {
        string.getCharacters_range(buffer, NSRange::new(0, len));
    }
    String::from_utf16(&units).ok()
}

fn input_string(object: &AnyObject) -> Option<String> {
    if let Some(string) = object.downcast_ref::<NSString>() {
        owned_string(string)
    } else {
        owned_string(&object.downcast_ref::<NSAttributedString>()?.string())
    }
}

fn editing_command(selector: &[u8]) -> Option<TextEditingCommand> {
    use TextEditingCommand::*;
    Some(match selector {
        b"moveLeft:" | b"moveBackward:" => MoveLeft { select: false },
        b"moveLeftAndModifySelection:" | b"moveBackwardAndModifySelection:" => {
            MoveLeft { select: true }
        }
        b"moveRight:" | b"moveForward:" => MoveRight { select: false },
        b"moveRightAndModifySelection:" | b"moveForwardAndModifySelection:" => {
            MoveRight { select: true }
        }
        b"moveUp:" => MoveUp { select: false },
        b"moveUpAndModifySelection:" => MoveUp { select: true },
        b"moveDown:" => MoveDown { select: false },
        b"moveDownAndModifySelection:" => MoveDown { select: true },
        b"moveWordLeft:" | b"moveWordBackward:" => MoveWordLeft { select: false },
        b"moveWordLeftAndModifySelection:" | b"moveWordBackwardAndModifySelection:" => {
            MoveWordLeft { select: true }
        }
        b"moveWordRight:" | b"moveWordForward:" => MoveWordRight { select: false },
        b"moveWordRightAndModifySelection:" | b"moveWordForwardAndModifySelection:" => {
            MoveWordRight { select: true }
        }
        b"moveToBeginningOfLine:" | b"moveToLeftEndOfLine:" | b"moveToBeginningOfParagraph:" => {
            MoveLineStart { select: false }
        }
        b"moveToBeginningOfLineAndModifySelection:"
        | b"moveToLeftEndOfLineAndModifySelection:"
        | b"moveToBeginningOfParagraphAndModifySelection:" => MoveLineStart { select: true },
        b"moveToEndOfLine:" | b"moveToRightEndOfLine:" | b"moveToEndOfParagraph:" => {
            MoveLineEnd { select: false }
        }
        b"moveToEndOfLineAndModifySelection:"
        | b"moveToRightEndOfLineAndModifySelection:"
        | b"moveToEndOfParagraphAndModifySelection:" => MoveLineEnd { select: true },
        b"moveToBeginningOfDocument:" => MoveDocumentStart { select: false },
        b"moveToBeginningOfDocumentAndModifySelection:" => MoveDocumentStart { select: true },
        b"moveToEndOfDocument:" => MoveDocumentEnd { select: false },
        b"moveToEndOfDocumentAndModifySelection:" => MoveDocumentEnd { select: true },
        b"deleteBackward:" | b"deleteBackwardByDecomposingPreviousCharacter:" => DeleteBackward,
        b"deleteForward:" => DeleteForward,
        b"deleteWordBackward:" => DeleteWordBackward,
        b"deleteWordForward:" => DeleteWordForward,
        b"deleteToBeginningOfLine:" | b"deleteToBeginningOfParagraph:" => DeleteToLineStart,
        b"deleteToEndOfLine:" | b"deleteToEndOfParagraph:" => DeleteToLineEnd,
        b"insertNewline:" | b"insertLineBreak:" | b"insertNewlineIgnoringFieldEditor:" => {
            InsertNewline
        }
        _ => return None,
    })
}

#[derive(Clone)]
pub(super) struct Session {
    client: Retained<TextClient>,
    pub(super) context: Retained<NSTextInputContext>,
    pub(super) id: TextInputSession,
}

impl Session {
    pub(super) fn new(window: &Native, snapshot: TextInputSnapshot) -> Self {
        let mtm = window.view.mtm();
        let id = snapshot.session;
        let client = TextClient::alloc(mtm).set_ivars(ClientState {
            shared: window.shared.clone(),
            text: RefCell::new(TextState::new(snapshot)),
            key_turn: RefCell::new(None),
            retired: Cell::new(false),
            discarding: Cell::new(false),
        });
        // SAFETY: The NSObject initializer takes ownership of an initialized
        // allocation on the main thread. All client ivars have been set.
        let client: Retained<TextClient> = unsafe { msg_send![super(client), init] };
        let context = NSTextInputContext::initWithClient(
            NSTextInputContext::alloc(mtm),
            ProtocolObject::from_ref(&*client),
        );
        Self {
            client,
            context,
            id,
        }
    }

    pub(super) fn publish(&self, snapshot: TextInputSnapshot) {
        let change = self.client.ivars().text.borrow_mut().set_snapshot(snapshot);
        if change.discard_mark {
            self.discard_mark();
        }
        if change.invalidate_coordinates {
            self.context.invalidateCharacterCoordinates();
        }
    }

    fn discard_mark(&self) {
        let state = self.client.ivars();
        state.discarding.set(true);
        self.context.discardMarkedText();
        state.discarding.set(false);
    }

    pub(super) fn retire(&self, report_cancel: bool) {
        if report_cancel && self.client.has_mark() {
            self.client.cancel_mark();
        }
        // Suppress callbacks that the old IME sends while deactivating its context.
        self.client.ivars().retired.set(true);
        self.context.discardMarkedText();
        self.context.deactivate();
    }

    pub(super) fn key(&self, event: &NSEvent, transition: &WindowEvent) -> bool {
        let WindowEvent::Key {
            code,
            modifiers,
            timestamp,
            ..
        } = *transition
        else {
            return false;
        };
        let state = self.client.ivars();
        *state.key_turn.borrow_mut() = Some(KeyTurn {
            code,
            modifiers,
            timestamp,
            edits: Vec::new(),
            claimed: false,
            unclaimed: false,
            discard_mark: false,
        });
        let handled = self.context.handleEvent(event);
        let Some(mut turn) = state.key_turn.borrow_mut().take() else {
            return handled;
        };
        if turn.discard_mark {
            self.discard_mark();
        }
        if !handled
            && !turn.claimed
            && !turn.unclaimed
            && !modifiers.control
            && !modifiers.meta
            && let Some(text) = event.characters().as_deref().and_then(owned_string)
            && !text.is_empty()
            && text
                .chars()
                .all(|ch| !ch.is_control() && !('\u{f700}'..='\u{f8ff}').contains(&ch))
        {
            // An input context can decline a plain press on some keyboard sources.
            // Use Cocoa's layout-produced characters, keeping text off the raw-key path.
            turn.edits
                .extend(state.text.borrow_mut().commit(text, MISSING, timestamp));
            turn.claimed = true;
        }
        state
            .shared
            .emit_input(turn.edits.into_iter().map(WindowEvent::TextInput));
        turn.claimed || (handled && !turn.unclaimed)
    }
}
