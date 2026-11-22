//! Orthogonal routes of the wires between two ports (`specs/panel-editor`, task 4).

/// Grid the middle segment of a route lies on: half a module, 5 units.
const GRID: f32 = 5.0;

/// A route and the place of the number of the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub points: Vec<(f32, f32)>,
    /// Y of the middle of the last vertical segment, where the number runs along the wire.
    pub label_y: f32,
}

/// How far the crossing of wire `k` of `n` that run side by side is moved so that the wires do not
/// lie on each other: when the wires go to the right the rightmost turns first (highest), when
/// to the left the leftmost does, so no two of them cross.
pub fn stagger(k: usize, n: usize, dx: f32) -> f32 {
    if n < 2 || dx.abs() < 0.01 {
        return 0.0;
    }
    let rank = if dx > 0.0 { n - 1 - k } else { k };
    (rank as f32 - ((n - 1) / 2) as f32) * GRID
}

/// The route of a wire that leaves `from` downwards and arrives at `to` from above.
///
/// When the two are on one vertical the wire is a straight line; otherwise it runs down from
/// `from`, across at the height `bend` (midway on the grid, moved by `stagger`, when `None`) and
/// down again to `to`.
pub fn drop(from: (f32, f32), to: (f32, f32), bend: Option<f32>, stagger: f32) -> Route {
    if (from.0 - to.0).abs() < 0.01 {
        return Route {
            points: vec![from, to],
            label_y: (from.1 + to.1) / 2.0,
        };
    }
    let middle = bend.unwrap_or_else(|| ((from.1 + to.1) / 2.0 / GRID).round() * GRID + stagger);
    Route {
        points: vec![from, (from.0, middle), (to.0, middle), to],
        label_y: (middle + to.1) / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_by_side_wires_cross_at_different_heights() {
        let right: Vec<f32> = (0..3).map(|k| stagger(k, 3, 20.0)).collect();
        assert_eq!(right, vec![5.0, 0.0, -5.0]);
        let left: Vec<f32> = (0..3).map(|k| stagger(k, 3, -20.0)).collect();
        assert_eq!(left, vec![-5.0, 0.0, 5.0]);
        assert_eq!(stagger(0, 1, 20.0), 0.0);
        assert_eq!(stagger(1, 3, 0.0), 0.0);
    }

    #[test]
    fn one_vertical_is_a_straight_line() {
        let r = drop((100.0, 10.0), (100.0, 90.0), Some(40.0), 0.0);
        assert_eq!(r.points, vec![(100.0, 10.0), (100.0, 90.0)]);
        assert_eq!(r.label_y, 50.0);
    }

    #[test]
    fn two_verticals_are_joined_across_midway_on_the_grid() {
        let r = drop((100.0, 10.0), (140.0, 93.0), None, 0.0);
        assert_eq!(
            r.points,
            vec![(100.0, 10.0), (100.0, 50.0), (140.0, 50.0), (140.0, 93.0)]
        );
        assert_eq!(r.label_y, 71.5);
    }

    #[test]
    fn the_bend_sets_the_height_of_the_crossing() {
        let r = drop((0.0, 0.0), (30.0, 100.0), Some(80.0), 0.0);
        assert_eq!(r.points[1], (0.0, 80.0));
        assert_eq!(r.points[2], (30.0, 80.0));
    }
}
