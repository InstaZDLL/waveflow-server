use super::*;

fn ids(count: usize) -> Vec<Uuid> {
    (0..count).map(|_| Uuid::new_v4()).collect()
}

#[test]
fn a_move_lands_where_it_was_asked_to_be_shown() {
    let tracks = ids(4);
    let visible = tracks.iter().copied().collect();
    let forward = moved(&tracks, &visible, 0, 2).unwrap();
    assert_eq!(forward, [tracks[1], tracks[2], tracks[0], tracks[3]]);
    let backward = moved(&tracks, &visible, 3, 0).unwrap();
    assert_eq!(backward, [tracks[3], tracks[0], tracks[1], tracks[2]]);
    let last = moved(&tracks, &visible, 1, 3).unwrap();
    assert_eq!(last, [tracks[0], tracks[2], tracks[3], tracks[1]]);
    assert_eq!(moved(&tracks, &visible, 2, 2).unwrap(), tracks);
    assert!(moved(&tracks, &visible, 4, 0).is_none());
    assert!(moved(&tracks, &visible, 0, 4).is_none());
}

/// A hidden entry keeps its place among the others, and the positions
/// are the shown ones: moving shown 0 to shown 1 passes the hidden entry
/// between them by, rather than landing on it.
#[test]
fn a_hidden_entry_neither_counts_nor_moves() {
    let tracks = ids(4);
    let hidden = tracks[1];
    let visible = tracks
        .iter()
        .copied()
        .filter(|track| *track != hidden)
        .collect();
    // Shown: 0, 2, 3.
    let order = moved(&tracks, &visible, 0, 1).unwrap();
    assert_eq!(order, [hidden, tracks[2], tracks[0], tracks[3]]);
    let to_end = moved(&tracks, &visible, 0, 2).unwrap();
    assert_eq!(to_end, [hidden, tracks[2], tracks[3], tracks[0]]);
    let to_front = moved(&tracks, &visible, 2, 0).unwrap();
    assert_eq!(to_front, [tracks[3], tracks[0], hidden, tracks[2]]);
}

/// The same track twice: the copy at the named position moves, and the
/// other stays.
#[test]
fn a_repeat_moves_one_copy() {
    let tracks = ids(2);
    let stored = vec![tracks[0], tracks[1], tracks[0]];
    let visible = tracks.iter().copied().collect();
    let order = moved(&stored, &visible, 2, 0).unwrap();
    assert_eq!(order, [tracks[0], tracks[0], tracks[1]]);
    let order = moved(&stored, &visible, 0, 2).unwrap();
    assert_eq!(order, [tracks[1], tracks[0], tracks[0]]);
}
