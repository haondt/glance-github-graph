use askama::Template;

pub struct GraphCell {
    pub date: String,
    pub count: u32,
    pub col: usize,
    pub row: usize,
    pub color: String,
    pub hover_text: String,
}

#[derive(Template)]
#[template(path = "stats.html")]
pub struct ContributionStatsTemplate<'a> {
    pub stats: &'a crate::ContributionStats,
    pub show_quartiles: bool,
    pub quartiles_string: String,
}

#[derive(Template)]
#[template(path = "svg_graph.svg")]
pub struct ContributionSvgGraphTemplate<'a> {
    pub stats: &'a crate::ContributionStats,
    pub max_count: u32,
    pub cells: Vec<GraphCell>,
    pub show_months: bool,
    pub svg_height: String,
    pub font_size: String,
    pub show_weekdays: bool,
    pub primary_color: String,
    pub color_shades: Vec<String>,
    pub month_labels: Vec<(usize, String)>,
    pub weekday_labels: Vec<(usize, &'static str)>,
    pub cell_radius: u32,
}

#[derive(Template)]
#[template(path = "graph.html")]
pub struct ContributionGraphHtmlTemplate<'a> {
    pub svg: ContributionSvgGraphTemplate<'a>,
    pub quartiles: String,
}

