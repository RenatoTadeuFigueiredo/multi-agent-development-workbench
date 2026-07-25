// Feature 015 — provider-native write tools under central policy.
package workbench

#ProviderNativeWritePolicy: {
	mode:      "disabled" | "approval-required"
	allowlist: [...string]
}

#Feature015: {
	default_mode: "disabled"
	claude_write_tools: ["Write", "Edit"]
	codex_write_items: ["file_change"]
}
