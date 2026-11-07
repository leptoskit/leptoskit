use wasm_bindgen::prelude::*;
mod user;

#[wasm_bindgen(start)]
pub fn start() {
	leptoskit::leptos::mount::mount_to_body(move || {
		let app = user::app();
		move || leptoskit::web::render(app())
	});
}
