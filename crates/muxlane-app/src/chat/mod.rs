//! Native Chat GPUI subsystem: composer, Markdown/diff renderers, syntax highlighting, and timeline view.
pub(crate) mod composer;
pub(crate) mod markdown;
pub(crate) mod syntax;
pub(crate) mod view;

pub(crate) use view::{ChatBackend, ChatView, ChatViewEvent};
