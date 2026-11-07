#![cfg(all(target_arch = "wasm32", not(feature = "guest")))]

use leptoskit_bridge::Error;
use leptoskit_plugin_device::Brand;
use leptoskit_plugin_device::Platform;
use leptoskit_plugin_device::Snapshot;
use leptoskit_plugin_device::web;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

// Controlled client hints shadow the browser's own on the navigator instance,
// so the tests do not depend on which browser runs them. `requested` records
// the hint names the plugin asked for.
#[wasm_bindgen(inline_js = r"
let requested = null;
function define(name, value) {
  Object.defineProperty(navigator, name, {configurable: true, get() { return value; }});
}
export function installHints(outcome) {
  requested = null;
  define('hardwareConcurrency', 8);
  define('maxTouchPoints', 5);
  define('userAgentData', {
    brands: [
      {brand: 'Chromium', version: '142'},
      {brand: 'Not_A Brand', version: ''},
      {brand: '', version: '1'},
      {version: '2'},
    ],
    mobile: true,
    platform: 'Android',
    getHighEntropyValues(names) {
      requested = Array.from(names);
      if (outcome === 'deny') {
        return Promise.reject(new DOMException('blocked', 'NotAllowedError'));
      }
      if (outcome === 'fail') return Promise.reject(new Error('broken'));
      return Promise.resolve({
        architecture: 'arm',
        bitness: '64',
        model: 'Pixel 9',
        platformVersion: '15.0.0',
        wow64: false,
        formFactors: ['Mobile', ''],
        fullVersionList: [{brand: 'Chromium', version: '142.0.7444.59'}],
      });
    },
  });
}
export function removeHints() {
  define('userAgentData', undefined);
  define('hardwareConcurrency', 1.5);
  define('maxTouchPoints', -1);
}
export function restoreNavigator() {
  for (const name of ['userAgentData', 'hardwareConcurrency', 'maxTouchPoints']) {
    delete navigator[name];
  }
}
export function requestedHints() { return requested; }
")]
extern "C" {
	#[wasm_bindgen(js_name = installHints)]
	fn install_hints(outcome: &str);
	#[wasm_bindgen(js_name = removeHints)]
	fn remove_hints();
	#[wasm_bindgen(js_name = restoreNavigator)]
	fn restore_navigator();
	#[wasm_bindgen(js_name = requestedHints)]
	fn requested_hints() -> JsValue;
}

fn web_section(snapshot: Snapshot) -> leptoskit_plugin_device::Web {
	match snapshot.platform {
		Platform::Web(web) => web,
		other => panic!("expected a web section, got {other:?}"),
	}
}

#[wasm_bindgen_test]
fn snapshot_reads_low_entropy_hints_without_requesting_more() {
	install_hints("allow");
	let snapshot = web::snapshot().unwrap();
	restore_navigator();

	assert_eq!(snapshot.os_name.as_deref(), Some("Android"));
	assert_eq!(snapshot.cpu_count.map(std::num::NonZeroU32::get), Some(8));
	assert_eq!(snapshot.model, None, "the model is a high-entropy hint");
	assert!(
		requested_hints().is_null(),
		"a snapshot never asks for more"
	);
	let encoded = snapshot.encode().unwrap();
	assert_eq!(Snapshot::decode(&encoded).unwrap(), snapshot);
	let web = web_section(snapshot);
	assert_eq!(web.mobile, Some(true));
	assert_eq!(web.max_touch_points, Some(5));
	assert_eq!(
		web.brands,
		[
			Brand {
				brand: "Chromium".into(),
				version: "142".into()
			},
			Brand {
				brand: "Not_A Brand".into(),
				version: String::new()
			},
		],
		"unnamed brands are skipped"
	);
}

#[wasm_bindgen_test]
fn snapshot_without_client_hints_leaves_them_unknown() {
	remove_hints();
	let snapshot = web::snapshot().unwrap();
	restore_navigator();

	assert_eq!(snapshot.os_name, None);
	assert_eq!(snapshot.cpu_count, None, "a fractional count is invalid");
	let web = web_section(snapshot);
	assert_eq!(web.mobile, None);
	assert_eq!(web.max_touch_points, None, "a negative count is invalid");
	assert_eq!(web.brands, []);
}

#[wasm_bindgen_test]
async fn high_entropy_values_are_requested_only_when_called() {
	install_hints("allow");
	let values = web::high_entropy_values().await;
	let requested = requested_hints();
	restore_navigator();

	let values = values.unwrap();
	assert_eq!(values.architecture.as_deref(), Some("arm"));
	assert_eq!(values.bitness.as_deref(), Some("64"));
	assert_eq!(values.model.as_deref(), Some("Pixel 9"));
	assert_eq!(values.platform_version.as_deref(), Some("15.0.0"));
	assert_eq!(values.wow64, Some(false));
	assert_eq!(values.form_factors, ["Mobile"]);
	assert_eq!(
		values.full_version_list,
		[Brand {
			brand: "Chromium".into(),
			version: "142.0.7444.59".into()
		}]
	);
	let requested = js_sys::Array::from(&requested)
		.iter()
		.map(|name| name.as_string().unwrap())
		.collect::<Vec<_>>();
	assert_eq!(requested, web::HIGH_ENTROPY_HINTS);
}

#[wasm_bindgen_test]
async fn high_entropy_refusals_and_absence_map_to_errors() {
	install_hints("deny");
	let denied = web::high_entropy_values().await;
	install_hints("fail");
	let failed = web::high_entropy_values().await;
	remove_hints();
	let missing = web::high_entropy_values().await;
	restore_navigator();

	assert_eq!(denied, Err(Error::PermissionDenied));
	assert_eq!(failed, Err(Error::Native));
	assert_eq!(missing, Err(Error::Unavailable));
}
