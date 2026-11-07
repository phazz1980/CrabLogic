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
