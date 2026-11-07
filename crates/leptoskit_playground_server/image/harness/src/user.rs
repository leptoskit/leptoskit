use leptoskit::prelude::*;

pub fn app() -> impl Fn() -> Element {
	|| View().child(Text("Hello, playground"))
}
