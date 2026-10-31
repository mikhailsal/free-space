use super::*;

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(STAGE_SCHEMA).unwrap();
    conn
}

#[test]
fn production_child_queries_use_compact_index() {
    use rusqlite::types::Value;
    let conn = db();
    for id in 1..=75 {
        insert(&conn, id, 9, b"same-\xff\0name");
    }
    for limit in [1, BATCH] {
        for cursor in [false, true] {
            let values = if cursor {
                vec![
                    Value::Integer(9),
                    Value::Blob(b"same-\xff\0name".to_vec()),
                    Value::Integer(32),
                ]
            } else {
                vec![Value::Integer(9)]
            };
            assert_index_plan(&conn, &child_sql(cursor, limit), &values, "raw_children");
        }
    }
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
fn tied_blob_names_paginate_across_batches_by_rowid() {
    let conn = db();
    let name = b"same-\xff\0name";
    for id in (1..=75).rev() {
        insert(&conn, id, 9, name);
    }
    insert(&conn, 100, 10, name);
    let mut budget = Budget::default();
    let mut pending = Pending::default();
    let mut cursor = None;
    let mut ids = Vec::new();
    while let Some(row) = take(&conn, 9, &mut cursor, &mut pending, &mut budget) {
        assert_eq!(row.name, name);
        ids.push(row.id);
    }
    assert_eq!(ids, (1..=75).collect::<Vec<_>>());
    assert_eq!(budget.selects, 4);
}

fn populate_nested_wide(conn: &Connection) -> Vec<u64> {
    // Each first child is the next directory: buffers remain pending at
    // every ancestor while a deep, wide traversal descends.
    let mut parents = vec![1000];
    for depth in 0..10u64 {
        let parent = parents[depth as usize];
        for i in 0..40u64 {
            insert(
                conn,
                depth * 100 + i + 1,
                parent,
                format!("{i:03}").as_bytes(),
            );
        }
        parents.push(depth * 100 + 1);
    }
    parents
}

#[test]
fn shared_budget_falls_back_and_releases_drained_capacity() {
    let conn = db();
    let parents = populate_nested_wide(&conn);
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
