//! Scale marker for issue #42: a 10,000-contact database stays usable.
//!
//! Ignored by default — seeding 10k contacts (plus ~2k companies, ~5k
//! opportunities, ~30k activities, ~5k tasks) writes tens of thousands of
//! transactions through the application seam and takes minutes, which CI must
//! not pay for. Run it deliberately, release mode, with the timings printed:
//!
//!   cargo test --release --test scale -- --ignored --nocapture
//!
//! Or seed once and re-measure against the file:
//!
//!   cargo run --release --bin seed-dev-db -- --database /tmp/scale.sqlite3
//!   SCALE_DB=/tmp/scale.sqlite3 cargo test --release --test scale -- --ignored --nocapture

use std::path::PathBuf;
use std::time::Instant;

use contractorcrm_lib::application::{
    get_timeline, list_contacts, list_opportunities, list_tasks, search_records, ListTasksRequest,
};
use contractorcrm_lib::seed::{seed_database, SeedOptions};
use contractorcrm_lib::storage::Storage;

/// Run `body` a few times and report the best and mean wall-clock milliseconds.
fn time_it(label: &str, runs: usize, mut body: impl FnMut() -> usize) {
    let mut samples = Vec::with_capacity(runs);
    let mut rows = 0;
    for _ in 0..runs {
        let started = Instant::now();
        rows = body();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let best = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    println!("{label}: best {best:.1} ms, mean {mean:.1} ms ({rows} rows)");
}

#[test]
#[ignore = "seeds a 10k-contact database; minutes, not seconds"]
fn ten_thousand_contacts_stay_usable() {
    // Reuse a pre-seeded file when one is offered, otherwise build a fresh one.
    let temporary = tempfile::tempdir().expect("temp dir");
    let (database, seeded) = match std::env::var("SCALE_DB") {
        Ok(path) => (PathBuf::from(path), true),
        Err(_) => (temporary.path().join("scale.sqlite3"), false),
    };

    if !seeded {
        let mut storage = Storage::open(&database).expect("open");
        storage
            .connection()
            .execute_batch("PRAGMA synchronous = NORMAL;")
            .expect("pragma");
        let started = Instant::now();
        let summary = seed_database(
            &mut storage,
            &SeedOptions {
                contacts: 10_000,
                seed: 42,
            },
            |phase, done, total| {
                if done == total {
                    println!("seeded {phase}: {total}");
                }
            },
        )
        .expect("seed");
        println!(
            "seed: {:.1} s ({} contacts, {} opportunities, {} activities)",
            started.elapsed().as_secs_f64(),
            summary.contacts,
            summary.opportunities,
            summary.activities
        );
    }

    // Cold open — the storage half of "startup to interactive".
    let started = Instant::now();
    let storage = Storage::open_existing(&database).expect("open existing");
    println!(
        "storage open: {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    let file_bytes = std::fs::metadata(&database).expect("metadata").len();
    println!("database size: {:.1} MiB", file_bytes as f64 / 1_048_576.0);

    time_it("list_contacts", 5, || {
        list_contacts(&storage, false).expect("contacts").len()
    });
    time_it("list_opportunities", 5, || {
        list_opportunities(&storage, false)
            .expect("opportunities")
            .len()
    });
    time_it("list_tasks (open)", 5, || {
        list_tasks(
            &storage,
            ListTasksRequest {
                status: Some("open".into()),
                ..ListTasksRequest::default()
            },
        )
        .expect("tasks")
        .len()
    });
    time_it("search_records \"vinyl\"", 5, || {
        search_records(&storage, "vinyl".into(), None, None)
            .expect("search")
            .len()
    });
    time_it("search_records \"whitaker\"", 5, || {
        search_records(&storage, "whitaker".into(), None, None)
            .expect("search")
            .len()
    });

    // The seeder loads its first contact with a long timeline; ids are
    // UUIDv7, so the smallest id is that first contact.
    let contacts = list_contacts(&storage, false).expect("contacts");
    assert_eq!(contacts.len(), 10_000, "seeded contact count");
    let busiest = contacts
        .iter()
        .map(|row| row.contact.id.clone())
        .min()
        .expect("a contact");
    let timeline_rows = get_timeline(&storage, "contact", &busiest, true)
        .expect("timeline")
        .len();
    assert!(
        timeline_rows >= 250,
        "the busy contact should carry a long timeline, got {timeline_rows}"
    );
    time_it("get_timeline (busiest contact)", 5, || {
        get_timeline(&storage, "contact", &busiest, true)
            .expect("timeline")
            .len()
    });
}
