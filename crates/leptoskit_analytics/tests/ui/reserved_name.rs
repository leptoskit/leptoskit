#[derive(leptoskit_analytics::Event)]
#[event(name = "$pageview")]
struct Viewed {
	path: String,
}

fn main() {}
