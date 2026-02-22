use crate::{Line, markdown::MdDocument};

pub struct Section {
    backend: String,
    lines: Vec<Line>,
}

pub struct SectionIterator {}

impl SectionIterator {
    pub fn new(doc: MdDocument<'_>, width: u16) -> Self {
        SectionIterator {}
    }
}
