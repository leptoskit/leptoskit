fn main() {
	if let Some(path) = std::env::var_os("LEPTOSKIT_RPC_SPEC_OUT") {
		std::fs::write(
            path,
            r#"{"leptoskit_rpc_spec":3,"title":"fixture","version":"0.1.0","functions":[{"name":"list_items","kind":"query","module_path":"verified_backend::api","struct_name":"ListItems","docs":"Lists items.","path":"/api/list_items","method":"POST","input":"Json","output":"Json","error_content_type":"text/plain","params":[{"name":"prefix","docs":"Only names starting with this.","schema":{"type":"string"},"required":false}],"output_schema":{"type":"array","items":{"type":"string"}},"internal":false,"deprecated":false,"tags":[]}],"channels":[{"name":"updates","module_path":"verified_backend::api","struct_name":"Updates","docs":"Live updates.","fields":["room"],"args_schema":{"type":"object","properties":{"room":{"type":"string"}},"required":["room"]},"message_schema":{"type":"string"},"presence_schema":{"type":"object","properties":{"name":{"type":"string"}}},"internal":false,"deprecated":false,"tags":[]}],"live_path":"/live","schemas":{}}"#,
        )
        .unwrap();

		return;
	}

	panic!("database/auth startup must not be reached during export");
}
