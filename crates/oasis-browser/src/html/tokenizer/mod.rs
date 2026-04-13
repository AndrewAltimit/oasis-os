//! WHATWG HTML tokenizer.
//!
//! Implements the tokenization state machine from the
//! [WHATWG HTML parsing specification][spec]. Consumes a UTF-8 `&str`
//! and emits a flat `Vec<Token>`.
//!
//! This is a practical subset covering the states needed for real-world
//! HTML: tags, attributes, comments, DOCTYPE, character references,
//! RAWTEXT (`<script>`, `<style>`), and RCDATA (`<title>`, `<textarea>`).
//! Malformed input is always handled gracefully -- the tokenizer never
//! panics.
//!
//! ## State machine overview
//!
//! ```text
//!   Data ──'<'──> TagOpen ──'/'──> EndTagOpen ──> TagName
//!     │              │                               │
//!     │              ├──'!'──> MarkupDeclarationOpen  ├──ws──> BeforeAttributeName
//!     │              │            ├──"--"──> Comment  │            │
//!     │              │            └──"DOCTYPE"──>...  │       AttributeName
//!     │              └──alpha──> TagName              │            │
//!     │                                              '>'     BeforeAttributeValue
//!     │                                            (emit)     │     │     │
//!     ├── CharacterReference (&#...; / &name;)               "     '   unquoted
//!     ├── RawText  (<script>, <style> -- no tag parsing)
//!     └── RcData   (<title>, <textarea> -- refs only)
//! ```
//!
//! Each character is consumed once, advancing through states until a
//! token is emitted. The `RawText` and `RcData` states suppress normal
//! tag recognition inside their respective elements.
//!
//! [spec]: https://html.spec.whatwg.org/multipage/parsing.html#tokenization

mod char_ref;
mod states;
mod types;

#[cfg(test)]
mod tests;

// Re-export public types so the module API is unchanged.
// These types are part of Token's public API (e.g. Token::Doctype(DoctypeToken),
// StartTagToken { attributes: Vec<Attribute> }) and are used by tree_builder tests.
#[allow(unused_imports)]
pub use types::{Attribute, DoctypeToken, EndTagToken, StartTagToken, Token};

use types::{DoctypeBuilder, State, TagBuilder};

// ---------------------------------------------------------------------------
// Diagnostic progress hook
// ---------------------------------------------------------------------------
//
// The PSP backend wires `set_tokenize_progress_hook` to its on-disk
// `eboot.log` writer so a synchronous `navigate_vfs` that hangs on
// pathological input is observable from the remote test harness.
// Set to a no-op pointer if not configured.

type ProgressFn = fn(u64, usize, usize, usize, u32);
type YieldFn = fn();
static PROGRESS_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static YIELD_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Install a tokenizer progress hook. Called every 2048 state-machine
/// iterations with `(iter_count, pos, input_len, tokens_emitted, state_id)`.
/// Embedders use this on real hardware to spot infinite loops or
/// pathological slowdowns inside `Tokenizer::tokenize`.
pub fn set_tokenize_progress_hook(hook: ProgressFn) {
    PROGRESS_HOOK.store(hook as usize, std::sync::atomic::Ordering::Release);
}

/// Install a cooperative yield hook fired every 2048 tokenize iters.
/// Lets PSP cooperatively yield CPU to the cmd_server thread so the
/// remote test harness stays responsive while a long synchronous
/// `navigate_vfs` is running.
pub fn set_tokenize_yield_hook(hook: YieldFn) {
    YIELD_HOOK.store(hook as usize, std::sync::atomic::Ordering::Release);
}

fn tokenize_progress_log(iter: u64, pos: usize, input_len: usize, tokens: usize, state_id: u32) {
    let raw = PROGRESS_HOOK.load(std::sync::atomic::Ordering::Acquire);
    if raw == 0 {
        return;
    }
    // SAFETY: raw is either 0 (handled above) or a function pointer
    // we previously stored from `set_tokenize_progress_hook`. The
    // signature matches `ProgressFn`.
    let hook: ProgressFn = unsafe { std::mem::transmute::<usize, ProgressFn>(raw) };
    hook(iter, pos, input_len, tokens, state_id);
}

fn tokenize_yield() {
    let raw = YIELD_HOOK.load(std::sync::atomic::Ordering::Acquire);
    if raw == 0 {
        return;
    }
    // SAFETY: raw is either 0 or a `YieldFn` we stored.
    let hook: YieldFn = unsafe { std::mem::transmute::<usize, YieldFn>(raw) };
    hook();
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// WHATWG HTML tokenizer.
///
/// Construct with [`Tokenizer::new`], then call [`Tokenizer::tokenize`] to
/// consume the input and produce a `Vec<Token>`.
pub struct Tokenizer {
    input: Vec<char>,
    pub(crate) pos: usize,
    pub(crate) state: State,
    pub(crate) return_state: State,
    pub(crate) current_tag: Option<TagBuilder>,
    pub(crate) current_comment: String,
    pub(crate) current_doctype: DoctypeBuilder,
    pub(crate) temp_buffer: String,
    pub(crate) last_start_tag: Option<String>,
    pub(crate) char_ref_code: u32,
}

impl Tokenizer {
    /// Create a new tokenizer over the given UTF-8 input.
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            state: State::Data,
            return_state: State::Data,
            current_tag: None,
            current_comment: String::new(),
            current_doctype: DoctypeBuilder::new(),
            temp_buffer: String::new(),
            last_start_tag: None,
            char_ref_code: 0,
        }
    }

    /// Consume the input and return the token stream.
    pub fn tokenize(&mut self) -> Vec<Token> {
        // Pre-reserve a generous capacity. Without this, Vec::push
        // doubles capacity (… 512 → 1024 → 2048 → 4096) and each
        // growth event copies the entire buffer. On PSP this thrashes
        // the small-block allocator and burns time the cmd_server
        // thread needs to stay responsive. ~16K tokens covers the
        // densest pages we care about (Wikipedia main page tokenises
        // to ~3.5K on desktop).
        let mut tokens: Vec<Token> = Vec::with_capacity(16 * 1024);
        // Diagnostic progress tracking. The PSP backend wires
        // `tokenize_progress_hook` so we can spot infinite loops or
        // slow stretches in the state machine on real hardware.
        let mut iter_count: u64 = 0;
        let mut last_pos: usize = usize::MAX;
        let mut stuck_count: u32 = 0;
        loop {
            iter_count += 1;
            // Sparse progress + cooperative yield. The yield hook
            // fires every 512 iters so the embedder can let other
            // threads run. Logging is rarer (every 4096 iters) so
            // the diagnostic I/O itself doesn't dominate runtime on
            // PSP where each vlog write is 3 kernel calls.
            if iter_count.is_multiple_of(512) {
                tokenize_yield();
            }
            if iter_count.is_multiple_of(4096) {
                tokenize_progress_log(
                    iter_count,
                    self.pos,
                    self.input.len(),
                    tokens.len(),
                    self.state as u32,
                );
            }
            // Detect infinite loops: pos must advance at least every
            // 100k state-machine iterations or we abort with EOF.
            if self.pos == last_pos {
                stuck_count += 1;
                if stuck_count > 100_000 {
                    tokenize_progress_log(
                        iter_count,
                        self.pos,
                        self.input.len(),
                        tokens.len(),
                        self.state as u32,
                    );
                    tokens.push(Token::Eof);
                    break;
                }
            } else {
                stuck_count = 0;
                last_pos = self.pos;
            }
            let token = self.next_token();
            let is_eof = token == Token::Eof;
            Self::push_coalesced(&mut tokens, token);
            if is_eof {
                break;
            }
        }
        tokens
    }

    /// Coalesce consecutive `Character` tokens.
    fn push_coalesced(tokens: &mut Vec<Token>, token: Token) {
        if let Token::Character(ref new_text) = token
            && let Some(Token::Character(prev)) = tokens.last_mut()
        {
            prev.push_str(new_text);
            return;
        }
        tokens.push(token);
    }

    // -- helpers ------------------------------------------------------------

    pub(crate) fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    pub(crate) fn consume(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    pub(crate) fn reconsume(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    /// Case-insensitive look-ahead.
    pub(crate) fn starts_with_ci(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        if self.pos + chars.len() > self.input.len() {
            return false;
        }
        chars
            .iter()
            .enumerate()
            .all(|(i, &expected)| self.input[self.pos + i].eq_ignore_ascii_case(&expected))
    }

    fn is_rawtext_element(name: &str) -> bool {
        matches!(
            name,
            "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript"
        )
    }

    fn is_rcdata_element(name: &str) -> bool {
        matches!(name, "title" | "textarea")
    }

    /// After emitting a start tag, transition to the appropriate
    /// content state if the tag is RAWTEXT or RCDATA.
    fn maybe_switch_content_state(&mut self, name: &str) {
        if Self::is_rawtext_element(name) {
            self.state = State::RawText;
        } else if Self::is_rcdata_element(name) {
            self.state = State::RcData;
        }
    }

    /// Emit a start/end tag from the current tag builder, updating
    /// `last_start_tag` and the state machine as needed.
    pub(crate) fn emit_current_tag(&mut self) -> Token {
        let Some(tag) = self.current_tag.take() else {
            // State machine invariant: current_tag is always Some here.
            // Return a harmless empty comment if the invariant is violated
            // (defensive: avoids panicking on malformed input).
            return Token::Comment(String::new());
        };
        let name = tag.name.clone();
        let is_start = !tag.is_end_tag;
        let tok = tag.into_token();
        if is_start {
            self.last_start_tag = Some(name.clone());
            self.maybe_switch_content_state(&name);
        }
        tok
    }

    // -- main dispatch ------------------------------------------------------

    /// Produce the next token. This drains `temp_buffer` when the
    /// character-reference sub-machine has finished, then resumes the
    /// normal state loop.
    fn next_token(&mut self) -> Token {
        loop {
            // Drain temp_buffer left over from a character reference
            // that returned to a data-like state. Must be checked
            // every iteration so the resolved text is emitted before
            // the next data character is consumed.
            if !self.temp_buffer.is_empty() && !self.state.is_char_ref() {
                let text = std::mem::take(&mut self.temp_buffer);
                return Token::Character(text);
            }

            match self.state {
                State::Data => {
                    if let Some(t) = self.state_data() {
                        return t;
                    }
                },
                State::TagOpen => {
                    if let Some(t) = self.state_tag_open() {
                        return t;
                    }
                },
                State::EndTagOpen => {
                    if let Some(t) = self.state_end_tag_open() {
                        return t;
                    }
                },
                State::TagName => {
                    if let Some(t) = self.state_tag_name() {
                        return t;
                    }
                },
                State::SelfClosingStartTag => {
                    if let Some(t) = self.state_self_closing() {
                        return t;
                    }
                },
                State::BeforeAttributeName => {
                    if let Some(t) = self.state_before_attr_name() {
                        return t;
                    }
                },
                State::AttributeName => {
                    if let Some(t) = self.state_attr_name() {
                        return t;
                    }
                },
                State::AfterAttributeName => {
                    if let Some(t) = self.state_after_attr_name() {
                        return t;
                    }
                },
                State::BeforeAttributeValue => {
                    if let Some(t) = self.state_before_attr_value() {
                        return t;
                    }
                },
                State::AttributeValueDoubleQuoted => {
                    if let Some(t) = self.state_attr_val_dq() {
                        return t;
                    }
                },
                State::AttributeValueSingleQuoted => {
                    if let Some(t) = self.state_attr_val_sq() {
                        return t;
                    }
                },
                State::AttributeValueUnquoted => {
                    if let Some(t) = self.state_attr_val_unquoted() {
                        return t;
                    }
                },
                State::AfterAttributeValueQuoted => {
                    if let Some(t) = self.state_after_attr_val_q() {
                        return t;
                    }
                },
                State::MarkupDeclarationOpen => {
                    if let Some(t) = self.state_markup_decl_open() {
                        return t;
                    }
                },
                State::CommentStart => {
                    if let Some(t) = self.state_comment_start() {
                        return t;
                    }
                },
                State::CommentStartDash => {
                    if let Some(t) = self.state_comment_start_dash() {
                        return t;
                    }
                },
                State::Comment => {
                    if let Some(t) = self.state_comment() {
                        return t;
                    }
                },
                State::CommentEndDash => {
                    if let Some(t) = self.state_comment_end_dash() {
                        return t;
                    }
                },
                State::CommentEnd => {
                    if let Some(t) = self.state_comment_end() {
                        return t;
                    }
                },
                State::Doctype => {
                    if let Some(t) = self.state_doctype() {
                        return t;
                    }
                },
                State::BeforeDoctypeName => {
                    if let Some(t) = self.state_before_doctype_name() {
                        return t;
                    }
                },
                State::DoctypeName => {
                    if let Some(t) = self.state_doctype_name() {
                        return t;
                    }
                },
                State::AfterDoctypeName => {
                    if let Some(t) = self.state_after_doctype_name() {
                        return t;
                    }
                },
                State::BogusComment => {
                    if let Some(t) = self.state_bogus_comment() {
                        return t;
                    }
                },
                State::CharacterReference => {
                    if let Some(t) = self.state_char_ref() {
                        return t;
                    }
                },
                State::NumericCharacterReference => {
                    if let Some(t) = self.state_numeric_char_ref() {
                        return t;
                    }
                },
                State::HexCharacterReferenceStart => {
                    if let Some(t) = self.state_hex_char_ref_start() {
                        return t;
                    }
                },
                State::HexCharacterReference => {
                    if let Some(t) = self.state_hex_char_ref() {
                        return t;
                    }
                },
                State::DecimalCharacterReference => {
                    if let Some(t) = self.state_dec_char_ref() {
                        return t;
                    }
                },
                State::NamedCharacterReference => {
                    if let Some(t) = self.state_named_char_ref() {
                        return t;
                    }
                },
                State::RawText => {
                    if let Some(t) = self.state_rawtext() {
                        return t;
                    }
                },
                State::RcData => {
                    if let Some(t) = self.state_rcdata() {
                        return t;
                    }
                },
            }
        }
    }
}
