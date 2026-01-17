use crate::document::SectionID;

#[derive(Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    None,
    Links(CursorPointer),
    Search(String, Option<CursorPointer>),
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
// Points to a LineExtra by Section id and LineExtra index.
pub struct CursorPointer {
    // The Section (line(s))
    pub id: SectionID,
    // The matched LineExtra part index
    pub index: usize,
}
