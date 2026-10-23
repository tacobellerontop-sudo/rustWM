//! Layout algorithms: pure functions from a window count and an area to rects.

use crate::Rect;

/// Equal-width columns across `area`, left to right. The last column absorbs
/// any rounding remainder so the columns cover the area exactly.
pub fn columns(count: usize, area: Rect) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let base = area.w / n;
    (0..n)
        .map(|i| {
            let w = if i == n - 1 {
                area.w - base * (n - 1)
            } else {
                base
            };
            Rect::new(area.x + base * i, area.y, w, area.h)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_windows_no_rects() {
        assert!(columns(0, Rect::new(0, 0, 100, 100)).is_empty());
    }

    #[test]
    fn three_columns_cover_area() {
        let rects = columns(3, Rect::new(0, 0, 1000, 500));
        assert_eq!(
            rects,
            vec![
                Rect::new(0, 0, 333, 500),
                Rect::new(333, 0, 333, 500),
                Rect::new(666, 0, 334, 500),
            ]
        );
    }
}
