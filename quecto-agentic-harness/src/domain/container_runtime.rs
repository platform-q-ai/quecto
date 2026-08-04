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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerScriptSet {
    pub create: String,
    pub exec: String,
    pub inspect: String,
    pub kill: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerScriptsConfig {
    pub default: Option<String>,
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
        let mode = obj.get("mode").and_then(|v| v.as_str()).unwrap_or("new");
        match mode {
            "new" => Ok(Self::New {
                repo: obj.get("repo").and_then(|v| v.as_str()).map(str::to_string),
                container_script: obj
                    .get("container_script")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            }),
            "existing" => {
                let by_ref = obj.get("ref").and_then(|v| v.as_str()).map(str::to_string);
                let by_name = obj.get("name").and_then(|v| v.as_str()).map(str::to_string);
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
        let set = config
            .scripts
            .get(name)
            .ok_or_else(|| format!("container script set '{name}' is not configured"))?;
        if set.create.is_empty()
            || set.exec.is_empty()
            || set.inspect.is_empty()
            || set.kill.is_empty()
        {
            return Err(format!("container script set '{name}' is incomplete"));
        }
        Ok(Some((name, set)))
    }
}

pub trait AgentLaunchBackend: Send + Sync {
    fn backend_name(&self) -> &'static str;
}

#[derive(Debug, Default)]
pub struct LocalProcessLaunchBackend;

impl AgentLaunchBackend for LocalProcessLaunchBackend {
    fn backend_name(&self) -> &'static str {
        "local"
    }
}
