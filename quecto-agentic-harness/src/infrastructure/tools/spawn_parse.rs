pub(super) fn parse_disable_tools(args: &serde_json::Value) -> Result<Vec<String>, String> {
    let mut tools: Vec<String> = Vec::new();
    let push_unique = |name: &str, tools: &mut Vec<String>| {
        if !tools.iter().any(|t| t == name) {
            tools.push(name.to_string());
        }
    };

    if let Some(v) = args.get("read_only").filter(|v| !v.is_null()) {
        if v.as_bool().ok_or("read_only must be a boolean")? {
            push_unique("write", &mut tools);
            push_unique("edit", &mut tools);
        }
    }
    if let Some(v) = args.get("disable_tools").filter(|v| !v.is_null()) {
        let arr = v
            .as_array()
            .ok_or("disable_tools must be an array of tool names")?;
        for entry in arr {
            let name = entry
                .as_str()
                .ok_or("disable_tools entries must be strings (tool names)")?;
            push_unique(name, &mut tools);
        }
    }
    Ok(tools)
}
