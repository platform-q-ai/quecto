use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnContainerRequest {
    Local,
    New {
        repo: Option<String>,
        container_script: Option<String>,
    },
    Existing {
        reference: ExistingContainerRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistingContainerRef {
    Ref(String),
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContainerScriptSet {
    pub create: String,
    pub exec: String,
    pub inspect: String,
    pub kill: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContainerScriptsConfig {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub scripts: HashMap<String, ContainerScriptSet>,
}

impl SpawnContainerRequest {
    pub fn parse(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::Local);
        };
        if value.is_null() || value == false {
            return Ok(Self::Local);
        }
        if value == true {
            return Ok(Self::New {
                repo: None,
                container_script: None,
            });
        }
        let obj = value
            .as_object()
            .ok_or("container must be a boolean or object")?;
        let string_field = |name: &str| -> Result<Option<String>, String> {
            obj.get(name)
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("container.{name} must be a string"))
                })
                .transpose()
        };
        let container_script_field = || -> Result<Option<String>, String> {
            match (obj.get("container_script"), obj.get("containerScript")) {
                (Some(_), Some(_)) => Err(
                    "container accepts either container_script or containerScript, not both"
                        .to_string(),
                ),
                (Some(v), None) => v
                    .as_str()
                    .map(|s| Some(s.to_string()))
                    .ok_or_else(|| "container.container_script must be a string".to_string()),
                (None, Some(v)) => v
                    .as_str()
                    .map(|s| Some(s.to_string()))
                    .ok_or_else(|| "container.containerScript must be a string".to_string()),
                (None, None) => Ok(None),
            }
        };
        let mode = string_field("mode")?.unwrap_or_else(|| "new".to_string());
        match mode.as_str() {
            "new" => Ok(Self::New {
                repo: string_field("repo")?,
                container_script: container_script_field()?,
            }),
            "existing" => {
                let by_ref = string_field("ref")?;
                let by_name = string_field("name")?;
                match (by_ref, by_name) {
                    (Some(r), None) => Ok(Self::Existing {
                        reference: ExistingContainerRef::Ref(r),
                    }),
                    (None, Some(n)) => Ok(Self::Existing {
                        reference: ExistingContainerRef::Name(n),
                    }),
                    (None, None) => {
                        Err("container mode 'existing' requires ref or name".to_string())
                    }
                    (Some(_), Some(_)) => Err(
                        "container mode 'existing' accepts either ref or name, not both"
                            .to_string(),
                    ),
                }
            }
            other => Err(format!("unsupported container mode '{other}'")),
        }
    }

    pub fn resolve_script<'a>(
        &'a self,
        config: &'a ContainerScriptsConfig,
    ) -> Result<Option<(&'a str, &'a ContainerScriptSet)>, String> {
        let Self::New {
            container_script, ..
        } = self
        else {
            return Ok(None);
        };
        let name = container_script
            .as_deref()
            .or(config.default.as_deref())
            .ok_or(
                "container spawn requires container_scripts.default or container.container_script",
            )?;
        Ok(Some(resolve_script_set(config, name)?))
    }

    pub fn resolve_default_script(
        config: &ContainerScriptsConfig,
    ) -> Result<(&str, &ContainerScriptSet), String> {
        let name = config
            .default
            .as_deref()
            .ok_or("existing container spawn requires container_scripts.default")?;
        resolve_script_set(config, name)
    }
}

fn resolve_script_set<'a>(
    config: &'a ContainerScriptsConfig,
    name: &'a str,
) -> Result<(&'a str, &'a ContainerScriptSet), String> {
    let set = config
        .scripts
        .get(name)
        .ok_or_else(|| format!("container script set '{name}' is not configured"))?;
    if set.create.is_empty() || set.exec.is_empty() || set.inspect.is_empty() || set.kill.is_empty()
    {
        return Err(format!("container script set '{name}' is incomplete"));
    }
    Ok((name, set))
}
