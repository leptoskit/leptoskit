import type {
	Frame,
	LineHeight,
	Transaction,
	WidgetProps,
} from "libleptoskit.so";
/** Yoga frames are logical vp; ArkUI must not independently reflow the tree. */
export class Widget {
	id: number;
	kind: string;
	props: WidgetProps;
	frame: Frame;
	children: Widget[] = [];

	constructor(id: number, kind: string, props: WidgetProps) {
		this.id = id;
		this.kind = kind;
		this.props = props;
		this.frame = { id: id, x: 0, y: 0, width: 0, height: 0 };
	}

	contentWidth(): number {
		let width = this.frame.width;
		this.children.forEach((child: Widget) => {
			width = Math.max(width, child.frame.x + child.frame.width);
		});

		return width;
	}

	contentHeight(): number {
		let height = this.frame.height;
		this.children.forEach((child: Widget) => {
			height = Math.max(height, child.frame.y + child.frame.height);
		});

		return height;
	}
}

/** Mutations keep widget identities. Snapshots publish one complete UI update. */
export class Scene {
	nodes: Map<number, Widget> = new Map();

	node(id: number): Widget {
		const node = this.nodes.get(id);

		if (node === undefined) {
			throw new Error(`Unknown render node ${id}`);
		}

		return node;
	}

	snapshot(node: Widget): Widget {
		const copy = new Widget(node.id, node.kind, node.props);
		copy.frame = node.frame;
		copy.children = node.children.map((child: Widget) => this.snapshot(child));

		return copy;
	}

	apply(transaction: Transaction): Widget | undefined {
		if (transaction.version !== 1) {
			throw new Error("Unsupported native render protocol");
		}

		transaction.mutations.forEach((change) => {
			if (change.Create !== undefined) {
				const created = change.Create;
				if (
					![
						"view",
						"text",
						"button",
						"text_input",
						"password_input",
						"scroll_view",
						"image",
						"rive",
					].includes(created.kind)
				) {
					throw new Error(`Unsupported HarmonyOS widget: ${created.kind}`);
				}
				if (this.nodes.has(created.id)) {
					throw new Error(`Duplicate render node ${created.id}`);
				}

				this.nodes.set(
					created.id,
					new Widget(created.id, created.kind, created.props),
				);
			} else if (change.UpdateProps !== undefined) {
				this.node(change.UpdateProps.id).props = change.UpdateProps.props;
			} else if (change.UpdateLayout !== undefined) {
				this.node(change.UpdateLayout.id).frame = change.UpdateLayout;
			} else if (change.InsertChild !== undefined) {
				const edge = change.InsertChild;
				const parent = this.node(edge.parent);
				const child = this.node(edge.child);
				parent.children = parent.children.filter((item: Widget) =>
					item.id !== child.id
				);
				parent.children.splice(edge.index, 0, child);
			} else if (change.RemoveChild !== undefined) {
				const edge = change.RemoveChild;
				const parent = this.node(edge.parent);
				parent.children = parent.children.filter((item: Widget) =>
					item.id !== edge.child
				);
			} else if (change.Delete !== undefined) {
				this.nodes.delete(change.Delete.id);
			} else {
				throw new Error("Unknown native render mutation");
			}
		});
		this.nodes.forEach((node: Widget) => {
			const source = node.props.source;
			if (
				source && !source.startsWith("file://") &&
				assetRawfile(source) === undefined
			) {
				throw new Error(
					"HarmonyOS images require an asset:// source or an application-private file URL",
				);
			}
		});

		return transaction.root === null
			? undefined
			: this.snapshot(this.node(transaction.root));
	}
}

const ASSET_SCHEME = "asset://";

/**
 * The rawfile an `asset://<path>` source names: `leptoskit build` copies the
 * app's `assets/` folder into the HAP's `rawfile/assets`. `undefined` for any
 * other source; an error for a path that would leave the folder.
 */
export function assetRawfile(source: string): string | undefined {
	if (!source.startsWith(ASSET_SCHEME)) {
		return undefined;
	}
	const path = source.slice(ASSET_SCHEME.length);
	if (
		path.length === 0 || path.includes("\\") || path.includes("\0") ||
		path.split("/").some((segment: string) =>
			segment === "" || segment === "." || segment === ".."
		)
	) {
		throw new Error(
			`${source} is not an asset path inside the app's assets folder`,
		);
	}
	return `assets/${path}`;
}

/** Convert the renderer's packed RGBA to ArkUI's ARGB value. */
export function color(
	rgba: number | null | undefined,
	fallback: number,
): number {
	return rgba === undefined || rgba === null
		? fallback
		: ((rgba >>> 8) | ((rgba & 255) << 24)) >>> 0;
}

/** Per-edge border stroke widths in the same logical units as Yoga frames. */
export interface BorderEdges {
	top: number;
	right: number;
	bottom: number;
	left: number;
}

/**
 * Resolve stroke widths the way Android and GTK do: prefer `border_widths`
 * from edge utilities, otherwise use a uniform `border_width` on every side.
 * Leaving a missing edge at zero matches the web (no boxing a partial border).
 */
export function borderEdges(
	props: { border_width?: number | null; border_widths?: BorderEdges | null },
): BorderEdges {
	if (props.border_widths !== undefined && props.border_widths !== null) {
		return {
			top: props.border_widths.top,
			right: props.border_widths.right,
			bottom: props.border_widths.bottom,
			left: props.border_widths.left,
		};
	}

	const width = props.border_width ?? 0;

	return { top: width, right: width, bottom: width, left: width };
}

/** Whether text should draw in the italic slant. Unset means upright. */
export function isItalic(italic: boolean | null | undefined): boolean {
	return italic === true;
}

/**
 * A line's height in the same logical units as the font size.
 *
 * `relative` wins when it is present: it is a multiple of `fontSize`.
 * Otherwise `fixed` is already in those units. A missing, non-finite, or
 * non-positive value leaves the font's own line height.
 */
export function lineHeightPoints(
	height: LineHeight | null | undefined,
	fontSize: number,
): number | undefined {
	if (height === undefined || height === null) {
		return undefined;
	}

	let points: number | undefined = undefined;
	if (height.relative !== undefined) {
		points = height.relative * fontSize;
	} else {
		points = height.fixed;
	}

	if (points === undefined || !Number.isFinite(points) || points <= 0) {
		return undefined;
	}

	return points;
}

/** The `autocomplete` tokens password managers read on every platform. */
const TEXT_CONTENT: string[] = [
	"username",
	"email",
	"current-password",
	"new-password",
];

/** What a text field holds: the first `autocomplete` token password managers read. */
export function textContent(
	autocomplete: string | null | undefined,
): string | undefined {
	if (autocomplete === undefined || autocomplete === null) {
		return undefined;
	}

	for (const token of autocomplete.toLowerCase().split(" ")) {
		if (TEXT_CONTENT.includes(token)) {
			return token;
		}
	}

	return undefined;
}
