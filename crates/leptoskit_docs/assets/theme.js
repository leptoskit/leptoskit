// Apply the saved theme and code style before first paint without requiring
// inline scripts.
{
	const root = document.documentElement;

	try {
		const theme = localStorage.getItem("lk-theme");

		if (theme === "light" || theme === "dark") {
			root.dataset.theme = theme;
		}
	} catch {
		// Storage can be unavailable; the system colour scheme remains usable.
	}

	// Component trees show as `view!` markup or as builder calls. Setting the
	// style at all also reveals the switch, which needs JavaScript; without
	// this script, samples show as markup. It is set once: any write to the
	// root's attribute restyles the whole page.
	let codeStyle = "markup";
	try {
		if (localStorage.getItem("lk-code-style") === "builder") {
			codeStyle = "builder";
		}
	} catch {
		// Storage can be unavailable; samples show as markup.
	}

	root.dataset.codeStyle = codeStyle;
}
