use serde::Deserialize;

/// Top-level document from `mutool draw -F stext.json`.
#[derive(Deserialize)]
pub struct StextDoc {
    pub pages: Vec<StextPage>,
}

#[derive(Deserialize)]
pub struct StextPage {
    pub blocks: Vec<StextBlock>,
}

#[derive(Deserialize)]
pub struct StextBlock {
    pub lines: Vec<StextLine>,
}

#[derive(Deserialize)]
pub struct StextLine {
    pub x: f64,
    pub y: f64,
    pub font: StextFont,
    pub text: String,
}

#[derive(Deserialize)]
pub struct StextFont {
    pub size: f64,
}
