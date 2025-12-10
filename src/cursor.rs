use crate::SourceID;

#[derive(Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    None,
    Links(CursorPointer),
    Search(SearchState, Option<CursorPointer>),
}

impl Cursor {
    pub fn pointer(&self) -> Option<&CursorPointer> {
        match &self {
            Cursor::None => None,
            Cursor::Links(pointer) => Some(pointer),
            Cursor::Search(_, pointer) => pointer.as_ref(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
// Points to a LineExtra by WidgetSource id and LinexExtra index.
pub struct CursorPointer {
    // The WidgetSource (line(s))
    pub id: SourceID,
    // The matched LinexExtra part index
    pub index: usize,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct SearchState {
    pub needle: String,
    pub accepted: bool,
}
