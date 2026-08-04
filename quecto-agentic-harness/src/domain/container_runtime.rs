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
        let string_field = |name: &str| -> Result<Option<String>, String> {
            obj.get(name)
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("container.{name} must be a string"))
                })
                .transpose()
        };
        let mode = string_field("mode")?.unwrap_or_else(|| "new".to_string());
        match mode.as_str() {
            "new" => Ok(Self::New {
                repo: string_field("repo")?,
                container_script: string_field("container_script")?,
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
