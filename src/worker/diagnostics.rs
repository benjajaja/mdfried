use cosmic_text::Color;
use image::Rgba;
use ratatui_image::{
    FontSize,
    picker::{Capability, Picker, ProtocolType},
};

use crate::setup::FontRenderer;

pub struct Diagnostics {
    pub protocol_type: ProtocolType,
    pub caps: Vec<Capability>,
    pub tmux_detected: bool,
    pub font_renderer: Option<FontDiagnostics>,
}

pub struct FontDiagnostics {
    pub font_size: FontSize,
    pub font_name: String,
    pub font_color: Color,
    pub background_color: Option<Rgba<u8>>,
}

impl Diagnostics {
    pub(crate) fn from_picker(picker: &Picker) -> Self {
        Self {
            protocol_type: picker.protocol_type(),
            caps: picker.capabilities().clone(),
            tmux_detected: picker.tmux_detected(),
            font_renderer: None,
        }
    }

    pub(crate) fn with_renderer(mut self, renderer: &FontRenderer) -> Self {
        self.font_renderer = Some(FontDiagnostics {
            font_size: renderer.font_size,
            font_name: renderer.font_name.clone(),
            font_color: renderer.font_color,
            background_color: renderer.background_color,
        });
        self
    }
}
