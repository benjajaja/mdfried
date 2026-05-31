use std::sync::Arc;

use mdfrier::MarkdownLink;
use ratatui::{layout::Size, text::Line};
use ratatui_image::{Resize, picker::Picker, sliced::SlicedProtocol};

use crate::error::Error;

use image::load_from_memory;

pub async fn render_with_cmd(
    cmd: &str,
    lines: &Vec<Line<'static>>,
    width: u16,
    max_height: u16,
    picker: Arc<Picker>,
) -> Result<(SlicedProtocol, Size, Size, MarkdownLink), Error> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let diagram = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let max_size = Size::new(width, max_height);

    let cmd = cmd.to_owned();
    let (sliced, size) = tokio::task::spawn_blocking(move || {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let Some(stdin) = child.stdin.as_mut() else {
            return Err(Error::Io(std::io::Error::other(
                "mermaid_command pipe error",
            )));
        };
        stdin.write_all(diagram.as_bytes())?;

        let output = child.wait_with_output()?;
        let dyn_img = load_from_memory(&output.stdout)?;
        let size = Resize::Fit(None).size_for(&dyn_img, picker.font_size(), max_size);
        let sliced = SlicedProtocol::new(&picker, dyn_img, Some(size))?;
        Ok::<_, Error>((sliced, size))
    })
    .await??;

    let link = MarkdownLink {
        url: String::new(),
        description: "mermaid".to_owned(),
    };
    Ok((sliced, size, max_size, link))
}

#[cfg(feature = "mermaid")]
pub mod internal {
    use super::*;
    use crate::document::svg_tree_to_rgba;
    use cosmic_text::fontdb::Database;
    use image::DynamicImage;
    use mermaid_rs_renderer::Theme;

    #[cfg(feature = "mermaid")]
    pub async fn render(
        lines: &Vec<Line<'static>>,
        width: u16,
        max_height: u16,
        fontdb: Arc<Database>,
        picker: Arc<Picker>,
    ) -> Result<(SlicedProtocol, Size, Size, MarkdownLink), Error> {
        let diagram = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let max_width_px = width as f32 * picker.font_size().width as f32;
        let max_size = Size::new(width, max_height);

        let (sliced, size) = tokio::task::spawn_blocking(move || {
            let dyn_img = render_image(&diagram, fontdb, max_width_px, None)?;
            let size = Resize::Fit(None).size_for(&dyn_img, picker.font_size(), max_size);
            let sliced = SlicedProtocol::new(&picker, dyn_img, Some(size))?;
            Ok::<_, Error>((sliced, size))
        })
        .await??;

        let link = MarkdownLink {
            url: String::new(),
            description: "mermaid".to_owned(),
        };
        Ok((sliced, size, max_size, link))
    }

    #[cfg(feature = "mermaid")]
    fn render_image(
        diagram: &str,
        fontdb: Arc<Database>,
        max_width_px: f32,
        background: Option<String>,
    ) -> Result<DynamicImage, Error> {
        use mermaid_rs_renderer::{LayoutConfig, compute_layout, parse_mermaid, render_svg};
        use resvg::usvg;

        let parsed = parse_mermaid(diagram).map_err(|err| Error::Mermaid(err.into()))?;

        const DEFAULT_BACKGROUND: &str = "#1E1E1E";
        let theme = dark_mermaid_theme(background.unwrap_or(DEFAULT_BACKGROUND.to_owned()));
        let config = LayoutConfig::default();
        let layout = compute_layout(&parsed.graph, &theme, &config);

        let svg = render_svg(&layout, &theme, &config);

        let options = usvg::Options {
            fontdb,
            ..Default::default()
        };
        let tree = usvg::Tree::from_data(svg.as_bytes(), &options)
            .map_err(|err| Error::Mermaid(err.into()))?;

        let svg_width = tree.size().width();
        if svg_width > max_width_px {
            log::warn!(
                "mermaid diagram too wide ({svg_width:.0}px > {max_width_px:.0}px), skipping render"
            );
            return Err(Error::MermaidTooBig);
        }

        svg_tree_to_rgba(tree)
    }

    #[cfg(feature = "mermaid")]
    fn dark_mermaid_theme(background: String) -> Theme {
        Theme {
            background,
            primary_color: "#2B2D40".to_owned(),
            primary_text_color: "#D4D4D4".to_owned(),
            primary_border_color: "#6B7AA8".to_owned(),
            line_color: "#7A8FA8".to_owned(),
            secondary_color: "#3A3820".to_owned(),
            tertiary_color: "#2B2D40".to_owned(),
            edge_label_background: "rgba(30,30,30,0.92)".to_owned(),
            cluster_background: "#2A2A18".to_owned(),
            cluster_border: "#8A8A30".to_owned(),
            sequence_actor_fill: "#2D2D2D".to_owned(),
            sequence_actor_border: "#888888".to_owned(),
            sequence_actor_line: "#666666".to_owned(),
            sequence_note_fill: "#3A3820".to_owned(),
            sequence_note_border: "#8A8A30".to_owned(),
            sequence_activation_fill: "#2D2D2D".to_owned(),
            sequence_activation_border: "#888888".to_owned(),
            text_color: "#D4D4D4".to_owned(),
            git_commit_label_color: "#D4D4D4".to_owned(),
            git_commit_label_background: "#2B2D40".to_owned(),
            git_tag_label_color: "#D4D4D4".to_owned(),
            git_tag_label_background: "#2B2D40".to_owned(),
            git_tag_label_border: "hsl(240, 40%, 40%)".to_owned(),
            pie_colors: [
                "hsl(240, 40%, 35%)".to_owned(),
                "hsl(60, 50%, 30%)".to_owned(),
                "hsl(280, 40%, 35%)".to_owned(),
                "hsl(180, 40%, 30%)".to_owned(),
                "hsl(20, 50%, 35%)".to_owned(),
                "hsl(150, 40%, 30%)".to_owned(),
                "hsl(320, 40%, 35%)".to_owned(),
                "hsl(200, 40%, 35%)".to_owned(),
                "hsl(0, 50%, 35%)".to_owned(),
                "hsl(100, 40%, 30%)".to_owned(),
                "hsl(40, 50%, 30%)".to_owned(),
                "hsl(260, 40%, 35%)".to_owned(),
            ],
            pie_title_text_color: "#D4D4D4".to_owned(),
            pie_section_text_color: "#D4D4D4".to_owned(),
            pie_legend_text_color: "#D4D4D4".to_owned(),
            pie_stroke_color: "#D4D4D4".to_owned(),
            pie_outer_stroke_color: "#888888".to_owned(),
            ..Theme::mermaid_default()
        }
    }
}
