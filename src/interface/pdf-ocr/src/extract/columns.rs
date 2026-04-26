use crate::tesseract::OcrToken;

/// Group OCR word tokens into visual rows by y coordinate.
/// Two tokens are in the same row if their vertical centers are within `row_tolerance` pixels.
pub fn cluster_into_rows(tokens: &[OcrToken]) -> Vec<Vec<OcrToken>> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<OcrToken> = tokens.to_vec();
    sorted.sort_by_key(|t| t.top + t.height / 2);

    // Use the median token height as the row tolerance (≈ half line height).
    let mut heights: Vec<i32> = sorted.iter().map(|t| t.height).collect();
    heights.sort_unstable();
    let median_h = heights[heights.len() / 2];
    let tolerance = (median_h / 2).max(4);

    let mut rows: Vec<Vec<OcrToken>> = Vec::new();
    for token in sorted {
        let center = token.top + token.height / 2;
        if let Some(last_row) = rows.last_mut() {
            let last_center = last_row[0].top + last_row[0].height / 2;
            if (center - last_center).abs() <= tolerance {
                last_row.push(token);
                continue;
            }
        }
        rows.push(vec![token]);
    }
    for row in rows.iter_mut() {
        row.sort_by_key(|t| t.left);
    }
    rows
}
