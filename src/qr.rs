//! QR code of a session link, rendered as SVG.

use qrcode::{Color, EcLevel, QrCode};

/// Quiet zone around the code, in modules. Scanners need at least four.
const QUIET: usize = 4;

#[derive(Debug, thiserror::Error)]
#[error("cannot build a QR code: {0}")]
pub struct QrError(String);

/// Dark modules as rows of booleans.
fn matrix(text: &str) -> Result<Vec<Vec<bool>>, QrError> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|e| QrError(e.to_string()))?;
    let width = code.width();
    let colors = code.to_colors();
    Ok(colors
        .chunks(width)
        .map(|row| row.iter().map(|c| *c == Color::Dark).collect())
        .collect())
}

/// Black modules on a white square with a quiet zone, so it scans in light and dark themes.
pub fn svg(text: &str) -> Result<String, QrError> {
    let rows = matrix(text)?;
    let size = rows.len() + 2 * QUIET;
    let mut path = String::new();
    for (y, row) in rows.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if row[x] {
                let start = x;
                while x < row.len() && row[x] {
                    x += 1;
                }
                path.push_str(&format!(
                    "M{} {}h{}v1h-{}z",
                    start + QUIET,
                    y + QUIET,
                    x - start,
                    x - start
                ));
            } else {
                x += 1;
            }
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" width=\"{size}\" height=\"{size}\" \
         shape-rendering=\"crispEdges\" role=\"img\" aria-label=\"QR code\">\
         <rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/><path fill=\"#000\" d=\"{path}\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the same modules as the SVG into a grayscale bitmap and decodes it.
    fn decode(text: &str) -> String {
        let rows = matrix(text).unwrap();
        let scale = 6;
        let size = (rows.len() + 2 * QUIET) * scale;
        let mut pixels = vec![255u8; size * size];
        for (y, row) in rows.iter().enumerate() {
            for (x, dark) in row.iter().enumerate() {
                if *dark {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            pixels[((y + QUIET) * scale + dy) * size + (x + QUIET) * scale + dx] =
                                0;
                        }
                    }
                }
            }
        }
        let mut img =
            rqrr::PreparedImage::prepare_from_greyscale(size, size, |x, y| pixels[y * size + x]);
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "exactly one QR code is found");
        let (_, content) = grids[0].decode().expect("decodes");
        content
    }

    #[test]
    fn the_code_decodes_to_the_link() {
        assert_eq!(decode("https://bulut.dev/k7m3q"), "https://bulut.dev/k7m3q");
        assert_eq!(
            decode("http://localhost:3030/whh3h"),
            "http://localhost:3030/whh3h"
        );
    }

    #[test]
    fn svg_draws_exactly_the_dark_modules() {
        let url = "https://bulut.dev/k7m3q";
        let rows = matrix(url).unwrap();
        let dark: usize = rows.iter().flatten().filter(|d| **d).count();
        let svg = svg(url).unwrap();
        assert!(svg.starts_with("<svg ") && svg.ends_with("</svg>"));
        let n = rows.len() + 2 * QUIET;
        assert!(svg.contains(&format!("viewBox=\"0 0 {n} {n}\"")));
        // Runs of dark modules are drawn as one rectangle each; their lengths add up to the dark count.
        let drawn: usize = svg
            .split('M')
            .skip(1)
            .map(|seg| {
                let h = seg.split('h').nth(1).unwrap();
                h.split('v').next().unwrap().parse::<usize>().unwrap()
            })
            .sum();
        assert_eq!(drawn, dark);
        assert!(svg.contains("fill=\"#fff\"") && svg.contains("fill=\"#000\""));
    }

    #[test]
    fn same_input_gives_same_output() {
        assert_eq!(
            svg("https://bulut.dev/k7m3q").unwrap(),
            svg("https://bulut.dev/k7m3q").unwrap()
        );
        assert_ne!(
            svg("https://bulut.dev/k7m3q").unwrap(),
            svg("https://bulut.dev/k7m3r").unwrap()
        );
    }
}
