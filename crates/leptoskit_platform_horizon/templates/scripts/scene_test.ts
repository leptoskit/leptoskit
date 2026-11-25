import { deepStrictEqual, equal, throws } from "node:assert/strict";
import { test } from "node:test";
import {
	formatColor,
	horizonTextStyle,
	horizonViewStyle,
	Scene,
} from "./scene.ts";

test("Horizon scene applies mutations, preserves identity, and computes scroll content size", () => {
	const scene = new Scene();
	const initial = scene.apply({
		version: 1,
		root: 0,
		mutations: [
			{ Create: { id: 0, kind: "scroll_view", props: { horizontal: false } } },
			{
				Create: {
					id: 1,
					kind: "text",
					props: { text: "Score: 10", font_size: 20, font_weight: 700 },
				},
			},
			{
				Create: {
					id: 2,
					kind: "button",
					props: { text: "Play", border_radius: 12 },
				},
			},
			{ InsertChild: { parent: 0, child: 1, index: 0 } },
			{ InsertChild: { parent: 0, child: 2, index: 1 } },
			{ UpdateLayout: { id: 0, x: 0, y: 0, width: 320, height: 240 } },
			{ UpdateLayout: { id: 1, x: 16, y: 12, width: 200, height: 32 } },
			{ UpdateLayout: { id: 2, x: 16, y: 60, width: 288, height: 48 } },
		],
	});

	equal(initial?.children.length, 2);
	equal(initial?.children[0].props.text, "Score: 10");
	equal(initial?.contentWidth(), 320);
	equal(initial?.contentHeight(), 240);

	const buttonStyle = horizonViewStyle(initial!.children[1]);
	equal(buttonStyle.left, 16);
	equal(buttonStyle.top, 60);
	equal(buttonStyle.borderRadius, 12);

	const textStyle = horizonTextStyle(initial!.children[0].props);
	equal(textStyle.fontSize, 20);
	equal(textStyle.fontWeight, "bold");

	const updated = scene.apply({
		version: 1,
		root: 0,
		mutations: [
			{ UpdateProps: { id: 1, props: { text: "Score: 20" } } },
			{ RemoveChild: { parent: 0, child: 2 } },
			{ Delete: { id: 2 } },
		],
	});

	deepStrictEqual(updated?.children.map((child) => child.id), [1]);
	equal(updated?.children[0].props.text, "Score: 20");
	equal(initial?.children[0].props.text, "Score: 10");
	equal(scene.apply({ version: 1, root: null, mutations: [] }), undefined);
});

test("Horizon scene rejects unsupported widgets, bad protocol versions, and remote image URLs", () => {
	throws(
		() => new Scene().apply({ version: 2, root: null, mutations: [] }),
		/protocol/,
	);
	throws(
		() =>
			new Scene().apply({
				version: 1,
				root: 0,
				mutations: [{ Create: { id: 0, kind: "text_input", props: {} } }],
			}),
		/Unsupported Meta Horizon widget: text_input/,
	);
	throws(
		() =>
			new Scene().apply({
				version: 1,
				root: 0,
				mutations: [{
					Create: {
						id: 0,
						kind: "image",
						props: { source: "https://example.com/a.png" },
					},
				}],
			}),
		/remote URLs/,
	);
	equal(formatColor({ r: 1, g: 0.5, b: 0, a: 1 }), "rgba(255, 128, 0, 1)");
});
