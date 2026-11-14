//! The insights engine (§10 of the design): the query types the planner
//! turns into SQL, and what it answers. Seeding and exact numbers here;
//! the SQL each query builds is snapshot-tested in `query_sql.rs`.

#![cfg(not(target_arch = "wasm32"))]

// Seeded in the commits that follow.
#[test]
fn the_engine_answers_a_seeded_trend() {
	// placeholder until the engine lands
}
