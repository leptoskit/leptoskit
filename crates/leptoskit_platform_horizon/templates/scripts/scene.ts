/**
 * Foreign-headless render transaction scene for Meta Horizon Worlds Custom UI (`horizon/ui`).
 *
 * Applies leptoskit `version: 1` render transactions (`Create`, `UpdateProps`,
 * `UpdateLayout`, `InsertChild`, `RemoveChild`, `Delete`) and translates Yoga
 * frames and widget props into `horizon/ui` `ViewStyle` and `TextStyle` objects.
 */

export interface Frame {
	id: number;
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface ColorValue {
	r: number;
	g: number;
	b: number;
	a: number;
}

export interface BorderEdges {
	top?: number;
	right?: number;
	bottom?: number;
	left?: number;
}

export interface WidgetProps {
	text?: string;
	value?: string;
	source?: string;
	accessibility_label?: string;
	background?: ColorValue | string;
	color?: ColorValue | string;
	border_color?: ColorValue | string;
	border_width?: number;
	border_widths?: BorderEdges;
	border_radius?: number;
	font_size?: number;
	font_weight?: number | string;
	font_style?: string;
	opacity?: number;
	disabled?: boolean;
	horizontal?: boolean;
}

export interface TransactionChange {
	Create?: { id: number; kind: string; props: WidgetProps };
	UpdateProps?: { id: number; props: WidgetProps };
	UpdateLayout?: Frame;
	InsertChild?: { parent: number; child: number; index: number };
	RemoveChild?: { parent: number; child: number };
	Delete?: { id: number };
}

export interface Transaction {
	version: number;
	root: number | null;
	mutations: TransactionChange[];
}

export const SUPPORTED_WIDGETS: readonly string[] = [
	"view",
	"text",
	"button",
	"pressable",
	"scroll_view",
	"image",
];

export interface HorizonViewStyle {
	position: "absolute";
	left: number;
	top: number;
	width: number;
	height: number;
	backgroundColor?: string;
	borderColor?: string;
	borderWidth?: number;
	borderRadius?: number;
	opacity?: number;
}

export interface HorizonTextStyle {
	color?: string;
	fontSize?: number;
	fontWeight?: "normal" | "bold" | "100" | "200" | "300" | "400" | "500" | "600" | "700" | "800" | "900";
}

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
		this.frame = { id, x: 0, y: 0, width: 0, height: 0 };
	}

	contentWidth(): number {
		let width = this.frame.width;
		for (const child of this.children) {
			width = Math.max(width, child.frame.x + child.frame.width);
		}
		return width;
	}

	contentHeight(): number {
		let height = this.frame.height;
		for (const child of this.children) {
			height = Math.max(height, child.frame.y + child.frame.height);
		}
		return height;
	}
}

/** Formats a leptoskit RGBA color value (0..1 or 0..255) or CSS string into a `horizon/ui` color string. */
export function formatColor(value: ColorValue | string | undefined): string | undefined {
	if (value === undefined) {
		return undefined;
	}
	if (typeof value === "string") {
		return value;
	}
	const scale = (channel: number): number => {
		const normalized = channel <= 1 ? Math.round(channel * 255) : Math.round(channel);
		return Math.max(0, Math.min(255, normalized));
	};
	const alpha = Math.max(0, Math.min(1, value.a));
	return `rgba(${scale(value.r)}, ${scale(value.g)}, ${scale(value.b)}, ${alpha})`;
}

/** Resolves a uniform border width from either `border_width` or per-edge `border_widths`. */
export function uniformBorderWidth(props: WidgetProps): number | undefined {
	if (props.border_width !== undefined) {
		return props.border_width;
	}
	if (props.border_widths !== undefined) {
		const edges = [
			props.border_widths.top ?? 0,
			props.border_widths.right ?? 0,
			props.border_widths.bottom ?? 0,
			props.border_widths.left ?? 0,
		];
		const max = Math.max(...edges);
		return max > 0 ? max : undefined;
	}
	return undefined;
}

/** Builds a `horizon/ui` `ViewStyle` positioned strictly from Yoga's computed frame. */
export function horizonViewStyle(node: Widget): HorizonViewStyle {
	const style: HorizonViewStyle = {
		position: "absolute",
		left: node.frame.x,
		top: node.frame.y,
		width: node.frame.width,
		height: node.frame.height,
	};
	const backgroundColor = formatColor(node.props.background);
	if (backgroundColor !== undefined) {
		style.backgroundColor = backgroundColor;
	}
	const borderColor = formatColor(node.props.border_color);
	if (borderColor !== undefined) {
		style.borderColor = borderColor;
	}
	const borderWidth = uniformBorderWidth(node.props);
	if (borderWidth !== undefined) {
		style.borderWidth = borderWidth;
	}
	if (node.props.border_radius !== undefined) {
		style.borderRadius = node.props.border_radius;
	}
	if (node.props.opacity !== undefined) {
		style.opacity = node.props.opacity;
	}
	return style;
}

/** Builds a `horizon/ui` `TextStyle` from leptoskit text props. */
export function horizonTextStyle(props: WidgetProps): HorizonTextStyle {
	const style: HorizonTextStyle = {};
	const textColor = formatColor(props.color);
	if (textColor !== undefined) {
		style.color = textColor;
	}
	if (props.font_size !== undefined) {
		style.fontSize = props.font_size;
	}
	if (props.font_weight !== undefined) {
		if (typeof props.font_weight === "number") {
			style.fontWeight = props.font_weight >= 600 ? "bold" : "normal";
		} else if (props.font_weight === "bold" || props.font_weight === "normal") {
			style.fontWeight = props.font_weight;
		}
	}
	return style;
}

/** Rejects remote URLs in `horizon/ui` Image sources; Horizon Worlds loads bundled TextureAsset IDs only. */
export function validateImageSource(source: string | undefined): void {
	if (source === undefined || source.length === 0) {
		return;
	}
	if (source.startsWith("http://") || source.startsWith("https://")) {
		throw new Error(`Horizon Worlds Image does not load remote URLs: ${source}`);
	}
}

/** Retains widget identities across mutations and publishes immutable snapshots for `horizon/ui`. */
export class Scene {
	nodes: Map<number, Widget> = new Map();

	node(id: number): Widget {
		const found = this.nodes.get(id);
		if (found === undefined) {
			throw new Error(`Unknown render node ${id}`);
		}
		return found;
	}

	snapshot(node: Widget): Widget {
		const copy = new Widget(node.id, node.kind, { ...node.props });
		copy.frame = { ...node.frame };
		copy.children = node.children.map((child) => this.snapshot(child));
		return copy;
	}

	apply(transaction: Transaction): Widget | undefined {
		if (transaction.version !== 1) {
			throw new Error("Unsupported native render protocol");
		}
		for (const change of transaction.mutations) {
			if (change.Create !== undefined) {
				const created = change.Create;
				if (!SUPPORTED_WIDGETS.includes(created.kind)) {
					throw new Error(`Unsupported Meta Horizon widget: ${created.kind}`);
				}
				if (this.nodes.has(created.id)) {
					throw new Error(`Duplicate render node ${created.id}`);
				}
				validateImageSource(created.props.source);
				this.nodes.set(created.id, new Widget(created.id, created.kind, created.props));
			} else if (change.UpdateProps !== undefined) {
				validateImageSource(change.UpdateProps.props.source);
				this.node(change.UpdateProps.id).props = change.UpdateProps.props;
			} else if (change.UpdateLayout !== undefined) {
				this.node(change.UpdateLayout.id).frame = change.UpdateLayout;
			} else if (change.InsertChild !== undefined) {
				const edge = change.InsertChild;
				const parent = this.node(edge.parent);
				const child = this.node(edge.child);
				parent.children = parent.children.filter((item) => item.id !== child.id);
				parent.children.splice(edge.index, 0, child);
			} else if (change.RemoveChild !== undefined) {
				const edge = change.RemoveChild;
				const parent = this.node(edge.parent);
				parent.children = parent.children.filter((item) => item.id !== edge.child);
			} else if (change.Delete !== undefined) {
				this.nodes.delete(change.Delete.id);
			} else {
				throw new Error("Unknown native render mutation");
			}
		}
		if (transaction.root === null) {
			return undefined;
		}
		return this.snapshot(this.node(transaction.root));
	}
}
