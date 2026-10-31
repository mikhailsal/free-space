use super::*;

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE raw(id INTEGER PRIMARY KEY,parent INTEGER,name BLOB NOT NULL,kind INTEGER,flags INTEGER,allocated INTEGER,apparent INTEGER,mtime INTEGER,nlink INTEGER,errors INTEGER,scanned_at INTEGER); CREATE INDEX raw_children ON raw(parent,name,id);").unwrap();
    conn
}

fn insert(conn: &Connection, id: u64, parent: u64, name: &[u8]) {
    conn.execute(
        "INSERT INTO raw VALUES (?1,?2,?3,0,0,0,0,0,1,0,0)",
        params![id as i64, parent as i64, name],
    )
    .unwrap();
}

fn take(
    conn: &Connection,
    parent: u64,
    cursor: &mut Option<(Vec<u8>, u64)>,
    pending: &mut Pending,
    budget: &mut Budget,
) -> Option<Raw> {
    let row = next(conn, parent, cursor, pending, budget).unwrap();
    assert!(budget.slots <= budget.max_slots);
    assert!(budget.bytes <= budget.max_bytes);
    if let Some(row) = &row {
        *cursor = Some((row.name.clone(), row.id));
    }
    row
}

#[test]
fn wide_raw_names_keep_cursor_order_and_reduce_selects() {
    let conn = db();
    // Synthetic SQLite names can exceed filesystem component limits.
    for i in 0..75u64 {
        let mut name = vec![0xff, (i / 3) as u8];
        name.extend(vec![b'x'; 300]);
        insert(&conn, 75 - i, 0, &name);
    }
    let mut expected = conn
        .prepare("SELECT id,name FROM raw ORDER BY name,id")
        .unwrap();
    let expected: Vec<(u64, Vec<u8>)> = expected
        .query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let mut budget = Budget::default();
    let mut pending = Pending::default();
    let mut cursor = None;
    let mut got = Vec::new();
    while let Some(row) = take(&conn, 0, &mut cursor, &mut pending, &mut budget) {
        got.push((row.id, row.name));
    }
    assert_eq!(got, expected);
    assert_eq!(budget.selects, 4);
    let mut baseline = Budget {
        max_slots: 0,
        ..Budget::default()
    };
    let mut pending = Pending::default();
    let mut cursor = None;
    let mut fallback = Vec::new();
    while let Some(row) = take(&conn, 0, &mut cursor, &mut pending, &mut baseline) {
        fallback.push((row.id, row.name));
    }
    assert_eq!(fallback, expected);
    assert_eq!(baseline.selects, 76);
    assert_eq!((budget.slots, budget.bytes), (0, 0));
}

#[test]
fn shared_budget_falls_back_and_releases_drained_capacity() {
    let conn = db();
    // Each first child is the next directory: buffers remain pending at
    // every ancestor while a deep, wide traversal descends.
    let mut parents = vec![1000];
    for depth in 0..10u64 {
        let parent = parents[depth as usize];
        for i in 0..40u64 {
            insert(
                &conn,
                depth * 100 + i + 1,
                parent,
                format!("{i:03}").as_bytes(),
            );
        }
        parents.push(depth * 100 + 1);
    }
    let mut budget = Budget {
        max_slots: usize::MAX,
        max_bytes: usize::MAX,
        ..Budget::default()
    };
    let mut pending: Vec<Pending> = (0..3).map(|_| Pending::default()).collect();
    let mut cursors: Vec<Option<(Vec<u8>, u64)>> = vec![None; 3];
    take(
        &conn,
        parents[0],
        &mut cursors[0],
        &mut pending[0],
        &mut budget,
    )
    .unwrap();
    let capacity = pending[0].rows.capacity();
    budget.max_slots = capacity * 2;
    take(
        &conn,
        parents[1],
        &mut cursors[1],
        &mut pending[1],
        &mut budget,
    )
    .unwrap();
    assert_eq!(
        budget.slots,
        pending[0].rows.capacity() + pending[1].rows.capacity()
    );
    assert!(!budget.can_buffer());
    let before = budget.selects;
    for _ in 0..3 {
        take(
            &conn,
            parents[2],
            &mut cursors[2],
            &mut pending[2],
            &mut budget,
        )
        .unwrap();
    }
    assert_eq!(budget.selects - before, 3);
    assert_eq!(pending[2].rows.capacity(), 0);
    while !pending[0].rows.is_empty() {
        take(
            &conn,
            parents[0],
            &mut cursors[0],
            &mut pending[0],
            &mut budget,
        )
        .unwrap();
    }
    assert_eq!(budget.slots, pending[1].rows.capacity());
    take(
        &conn,
        parents[2],
        &mut cursors[2],
        &mut pending[2],
        &mut budget,
    )
    .unwrap();
    assert_eq!(
        budget.slots,
        pending[1].rows.capacity() + pending[2].rows.capacity()
    );
    for p in &mut pending {
        if p.rows.capacity() != 0 {
            p.release(&mut budget);
        }
    }
    assert_eq!((budget.slots, budget.bytes), (0, 0));
}

#[test]
fn byte_limit_discards_prefetch_without_losing_oversized_row() {
    let conn = db();
    insert(&conn, 1, 0, b"a");
    insert(&conn, 2, 0, &vec![b'b'; 2000]);
    insert(&conn, 3, 0, b"c");
    let mut budget = Budget {
        max_bytes: BATCH * size_of::<Raw>() + ALLOC_OVERHEAD + 200,
        ..Budget::default()
    };
    let mut pending = Pending::default();
    let mut cursor = None;
    let mut ids = Vec::new();
    while let Some(row) = take(&conn, 0, &mut cursor, &mut pending, &mut budget) {
        ids.push(row.id);
    }
    assert_eq!(ids, vec![1, 2, 3]);
    assert_eq!((budget.slots, budget.bytes), (0, 0));
}

#[test]
fn byte_overflow_in_equal_name_ties_resumes_from_consumed_id() {
    let conn = db();
    let name = vec![0xff; 300]; // Synthetic SQL name; not a filesystem component.
    for id in 1..=7 {
        insert(&conn, id, 0, &name);
    }
    let capacity = VecDeque::<Raw>::with_capacity(BATCH).capacity();
    let mut budget = Budget {
        max_bytes: capacity * size_of::<Raw>() + ALLOC_OVERHEAD + 2 * (name.len() + ALLOC_OVERHEAD),
        ..Budget::default()
    };
    let mut pending = Pending::default();
    let mut cursor = None;
    let first = take(&conn, 0, &mut cursor, &mut pending, &mut budget).unwrap();
    assert_eq!(pending.rows.len(), 1); // Exactly two tied names fit per refill.
    let mut ids = vec![first.id];
    while let Some(row) = take(&conn, 0, &mut cursor, &mut pending, &mut budget) {
        ids.push(row.id);
    }
    assert_eq!(ids, (1..=7).collect::<Vec<_>>());
    assert!(budget.selects > 2);
    assert_eq!((budget.slots, budget.bytes), (0, 0));
}

#[test]
fn eviction_prefers_shallow_ancestor_and_resumes_cursor() {
    use super::super::{Frame, evict_for_directory};
    fn frame(raw: u64, kind: Kind) -> Frame {
        Frame {
            raw,
            id: raw,
            kind,
            cursor: None,
            path_len: 0,
            agg: SubtreeAgg::of(&Node::default()),
            pending: Pending::default(),
        }
    }
    let conn = db();
    for parent in 0..2u64 {
        for i in 0..4u64 {
            insert(
                &conn,
                parent * 100 + i + 1,
                parent,
                format!("{i:03}").as_bytes(),
            );
        }
    }
    let mut budget = Budget {
        max_slots: usize::MAX,
        max_bytes: usize::MAX,
        ..Budget::default()
    };
    let mut stack = vec![
        frame(0, Kind::Dir),
        frame(1, Kind::Dir),
        frame(2, Kind::File),
    ];
    for frame in &mut stack[..2] {
        take(
            &conn,
            frame.raw,
            &mut frame.cursor,
            &mut frame.pending,
            &mut budget,
        )
        .unwrap();
    }
    budget.max_slots = budget.slots;
    evict_for_directory(&mut stack, &mut budget);
    assert!(!stack[0].pending.is_empty()); // Leaf lookups do not evict.
    stack[2].kind = Kind::Dir;
    evict_for_directory(&mut stack, &mut budget);
    assert!(stack[0].pending.is_empty());
    assert!(!stack[1].pending.is_empty());
    let root = &mut stack[0];
    let row = take(
        &conn,
        root.raw,
        &mut root.cursor,
        &mut root.pending,
        &mut budget,
    )
    .unwrap();
    assert_eq!(row.id, 2);
}
