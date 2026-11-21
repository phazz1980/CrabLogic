use super::*;

#[test]
fn forward_route_has_two_bends() {
    let r = route(Pos2::new(0.0, 0.0), Pos2::new(100.0, 40.0), 12.0, None);
    assert_eq!(r.len(), 4);
    assert_eq!(r[1], Pos2::new(50.0, 0.0));
}

#[test]
fn bend_moves_the_middle_segment() {
    let r = route(
        Pos2::new(0.0, 0.0),
        Pos2::new(100.0, 40.0),
        12.0,
        Some(30.0),
    );
    assert_eq!((r[1].x, r[2].x), (30.0, 30.0));
    // Never leaves the span between the ports.
    let r = route(
        Pos2::new(0.0, 0.0),
        Pos2::new(100.0, 40.0),
        12.0,
        Some(500.0),
    );
    assert_eq!(r[1].x, 98.0);
    let r = route(Pos2::new(100.0, 0.0), Pos2::new(0.0, 0.0), 12.0, Some(77.0));
    assert_eq!((r[2].y, r[3].y), (77.0, 77.0));
}

#[test]
fn backward_route_goes_around() {
    let r = route(Pos2::new(100.0, 0.0), Pos2::new(0.0, 0.0), 12.0, None);
    assert_eq!(r.len(), 6);
    assert!(r[2].y > 0.0);
}

#[test]
fn rounding_keeps_ends() {
    let pts = [
        Pos2::new(0.0, 0.0),
        Pos2::new(50.0, 0.0),
        Pos2::new(50.0, 40.0),
    ];
    let r = rounded(&pts, 6.0);
    assert_eq!(r.first(), Some(&pts[0]));
    assert_eq!(r.last(), Some(&pts[2]));
    assert!(r.len() > 3);
}

fn line(to: &str, from: &str, pts: &[(f32, f32)]) -> (PortRef, PortRef, Vec<Pos2>) {
    (
        to.parse().unwrap(),
        from.parse().unwrap(),
        pts.iter().map(|&(x, y)| Pos2::new(x, y)).collect(),
    )
}

#[test]
fn the_nearest_link_under_the_pointer_is_chosen() {
    let lines = [
        line("b.in", "a.out", &[(0.0, 0.0), (100.0, 0.0)]),
        line("d.in", "c.out", &[(0.0, 6.0), (100.0, 6.0)]),
    ];
    let pick =
        |p: Pos2, skip: Option<&str>| link_under(&lines, p, 10.0, skip).map(|t| t.to_string());
    assert_eq!(pick(Pos2::new(50.0, 1.0), None).as_deref(), Some("b.in"));
    assert_eq!(pick(Pos2::new(50.0, 5.0), None).as_deref(), Some("d.in"));
    // Too far from both.
    assert_eq!(pick(Pos2::new(50.0, 40.0), None), None);
    // Links of the moved block do not count.
    assert_eq!(
        pick(Pos2::new(50.0, 1.0), Some("a")).as_deref(),
        Some("d.in")
    );
}
