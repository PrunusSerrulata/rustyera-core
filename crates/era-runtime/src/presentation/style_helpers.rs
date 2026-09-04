fn replace_project_default_style(
    runs: &mut [DisplayRun],
    previous: &TextStyle,
    next: &TextStyle,
) -> bool {
    let mut changed = false;
    for run in runs {
        match run {
            DisplayRun::Text { style, .. }
            | DisplayRun::TextLayout { style, .. }
            | DisplayRun::Separator { style, .. } => {
                changed |= replace_matching_style_defaults(style, previous, next);
            }
            DisplayRun::Button {
                runs, hover_style, ..
            } => {
                changed |= replace_project_default_style(runs, previous, next);
                if let Some(style) = hover_style {
                    changed |= replace_matching_style_defaults(style, previous, next);
                }
            }
            DisplayRun::ColumnCell { content, .. } => {
                changed |= replace_project_default_style(content, previous, next);
            }
            DisplayRun::HtmlDocument { .. }
            | DisplayRun::Image { .. }
            | DisplayRun::Shape { .. }
            | DisplayRun::Space { .. } => {}
        }
    }
    changed
}

fn replace_matching_style_defaults(
    style: &mut TextStyle,
    previous: &TextStyle,
    next: &TextStyle,
) -> bool {
    let mut changed = false;
    if style.font_family == previous.font_family && style.font_family != next.font_family {
        style.font_family.clone_from(&next.font_family);
        changed = true;
    }
    if style.font_millipixels == previous.font_millipixels
        && style.font_millipixels != next.font_millipixels
    {
        style.font_millipixels = next.font_millipixels;
        changed = true;
    }
    if style.foreground == previous.foreground && style.foreground != next.foreground {
        style.foreground = next.foreground;
        changed = true;
    }
    changed
}

pub(super) fn line_has_text_background(runs: &[DisplayRun]) -> bool {
    runs.iter().any(run_has_text_background)
}

fn run_has_text_background(run: &DisplayRun) -> bool {
    match run {
        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
            !text.trim().is_empty()
        }
        DisplayRun::Button { runs, .. } => line_has_text_background(runs),
        DisplayRun::ColumnCell { content, .. } => line_has_text_background(content),
        DisplayRun::HtmlDocument { document } => html_nodes_have_text(&document.nodes),
        DisplayRun::Separator { pattern, .. } => !pattern.trim().is_empty(),
        DisplayRun::Image { .. } | DisplayRun::Shape { .. } | DisplayRun::Space { .. } => false,
    }
}

fn html_nodes_have_text(nodes: &[erabasic_html::HtmlNode]) -> bool {
    nodes.iter().any(|node| match node {
        erabasic_html::HtmlNode::Text { text, .. } => !text.trim().is_empty(),
        erabasic_html::HtmlNode::Element { children, .. } => html_nodes_have_text(children),
    })
}

