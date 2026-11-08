//! The soundboard example's compiled guest, in the real interpreter, driving
//! the engine: a press loads and plays a sound the mixer then renders, and the
//! voice's end reaches the screen.
#![cfg(not(target_arch = "wasm32"))]

use leptoskit_bridge::AppContext;
use leptoskit_bridge::Plugin;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_plugin_audio::Audio;
use leptoskit_plugin_audio::METHODS;
use leptoskit_plugin_audio::NAME;
use leptoskit_plugin_audio::Rendering;
use leptoskit_plugin_audio::offline;
use leptoskit_wasm::WasmHost;

const RATE: u32 = 48_000;

#[test]
#[ignore = "run test:plugins:guest to compile and load the soundboard guest"]
fn the_soundboard_plays_a_sound_on_a_press_and_reports_its_end() {
	let path = std::env::var("LEPTOSKIT_SOUNDBOARD_GUEST").expect("guest wasm path");
	let mut host =
		WasmHost::headless(&std::fs::read(path).expect("guest wasm")).expect("the guest loads");
	let plugin = host
		.plugins()
		.register(NAME, METHODS.iter().copied())
		.unwrap();

	for method in METHODS {
		host.plugins().grant(plugin, *method, true).unwrap();
	}

	let directory = std::env::temp_dir();
	let (mut audio, mut mixer) = offline(&AppContext::new(directory.clone(), directory), RATE);
	host.start(320.0, 480.0).expect("the soundboard mounts");
	serve(&mut host, &mut audio);
	assert_eq!(
		texts(&host),
		["Ready", "Strike", "Count endings", "Endings: 0"]
	);
	assert!(peak(&render(&mut mixer, 0.05)) < 1e-6);

	press(&mut host, "Strike");
	serve(&mut host, &mut audio);
	assert_eq!(texts(&host)[0], "Playing");
	// The tone peaks at 0.5, played at 0.8.
	let sound = peak(&render(&mut mixer, 0.2));
	assert!((sound - 0.4).abs() < 0.02, "peak {sound}");

	render(&mut mixer, 0.5);
	press(&mut host, "Count endings");
	serve(&mut host, &mut audio);
	assert_eq!(texts(&host)[3], "Endings: 1");
	assert!(peak(&render(&mut mixer, 0.05)) < 1e-6);
}

/// Answers the guest's calls with the plugin, as a generated host does.
fn serve(host: &mut WasmHost<HeadlessBackend>, audio: &mut Audio) {
	while let Some(request) = host.plugins().next_request() {
		let reply = audio.invoke(request.method, request.payload);
		host.plugins().complete(request.id, reply).unwrap();
		host.wake_plugins().expect("the guest takes the reply");
	}
}

fn render(mixer: &mut Rendering, seconds: f32) -> Vec<f32> {
	let mut output = vec![0.0; (seconds * RATE as f32) as usize * 2];
	mixer.render(&mut output, 2);
	output
}

fn peak(samples: &[f32]) -> f32 {
	samples
		.iter()
		.fold(0.0, |peak, sample| peak.max(sample.abs()))
}

/// The text of each child of the root, in order.
fn texts(host: &WasmHost<HeadlessBackend>) -> Vec<String> {
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	surface
		.children(root)
		.iter()
		.map(|child| surface.props(*child).text.clone().unwrap_or_default())
		.collect()
}

fn press(host: &mut WasmHost<HeadlessBackend>, label: &str) {
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	let button = *surface
		.children(root)
		.iter()
		.find(|&&child| {
			surface.kind(child) == NativeKind::Button
				&& surface.props(child).text.as_deref() == Some(label)
		})
		.expect("the button");
	host.dispatch(button, &NativeEvent::Press)
		.expect("the press reaches the guest");
}
