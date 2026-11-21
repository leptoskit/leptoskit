/**
 * Meta Horizon Worlds Custom UI host (`horizon/ui`) for __APP_NAME__.
 *
 * Attach this script to a Custom UI Gizmo in the Meta Horizon Worlds Desktop Editor.
 * Feed serialized leptoskit `version: 1` render transactions via `applyTransactionJson`
 * and listen to `LeptosKitPressEvent` for user presses.
 */

import * as hz from "horizon/core";
import {
	Binding,
	Image,
	ImageSource,
	Pressable,
	ScrollView,
	Text,
	UIComponent,
	UINode,
	View,
} from "horizon/ui";
import {
	horizonTextStyle,
	horizonViewStyle,
	Scene,
	type Transaction,
	type Widget,
} from "./scene";

export const LeptosKitPressEvent = new hz.LocalEvent<{ nodeId: number; player: hz.Player }>(
	"LeptosKitPressEvent",
);

export const LeptosKitTransactionEvent = new hz.LocalEvent<{ json: string }>(
	"LeptosKitTransactionEvent",
);

export class LeptosKitHost extends UIComponent<typeof LeptosKitHost> {
	static propsDefinition = {
		/** Optional initial serialized render transaction JSON. */
		initialTransaction: { type: hz.PropTypes.String, default: "" },
	};

	private readonly scene = new Scene();
	private readonly rootNode = new Binding<Widget | undefined>(undefined);
	private readonly textureAssets = new Map<string, hz.TextureAsset>();

	registerTexture(key: string, asset: hz.TextureAsset): void {
		this.textureAssets.set(key, asset);
	}

	applyTransaction(transaction: Transaction): void {
		const root = this.scene.apply(transaction);
		this.rootNode.set(root);
	}

	applyTransactionJson(json: string): void {
		const parsed = JSON.parse(json) as Transaction;
		this.applyTransaction(parsed);
	}

	start(): void {
		this.connectLocalEvent(this.entity, LeptosKitTransactionEvent, ({ json }) => {
			this.applyTransactionJson(json);
		});
		const initial = this.props.initialTransaction.trim();
		if (initial.length > 0) {
			this.applyTransactionJson(initial);
		}
	}

	initializeUI(): UINode {
		return UINode.if(
			this.rootNode.derive((root) => root !== undefined),
			this.rootNode.derive((root) =>
				root === undefined ? View({}) : this.renderWidget(root),
			),
		);
	}

	private renderWidget(node: Widget): UINode {
		const viewStyle = horizonViewStyle(node);
		const children = node.children.map((child) => this.renderWidget(child));

		switch (node.kind) {
			case "text":
				return Text({
					text: node.props.text ?? "",
					style: {
						...viewStyle,
						...horizonTextStyle(node.props),
					},
				});

			case "button":
			case "pressable":
				return Pressable({
					style: viewStyle,
					disabled: node.props.disabled ?? false,
					onClick: (player: hz.Player) => {
						this.sendLocalEvent(this.entity, LeptosKitPressEvent, {
							nodeId: node.id,
							player,
						});
					},
					children:
						children.length > 0
							? children
							: node.props.text !== undefined
								? [
										Text({
											text: node.props.text,
											style: horizonTextStyle(node.props),
										}),
									]
								: [],
				});

			case "scroll_view":
				return ScrollView({
					style: viewStyle,
					horizontal: node.props.horizontal ?? false,
					contentContainerStyle: {
						width: node.contentWidth(),
						height: node.contentHeight(),
					},
					children,
				});

			case "image": {
				const asset = node.props.source ? this.textureAssets.get(node.props.source) : undefined;
				if (asset === undefined) {
					return View({ style: viewStyle, children });
				}
				return Image({
					source: ImageSource.fromTextureAsset(asset),
					style: viewStyle,
				});
			}

			default:
				return View({
					style: viewStyle,
					children,
				});
		}
	}
}

UIComponent.register(LeptosKitHost);
