//! `ListModels` against fake ports only (#1845): the listing describes the
//! generation the call published, a failed source is reported rather than
//! listed over, and dropped records surface as diagnostics.

use std::sync::{Arc, Mutex};

use super::*;
use crate::application::catalogue::ports::LoadedCatalogueInputs;
use crate::application::catalogue::{
    CatalogueSource, CatalogueSourceError, CredentialStatusPort, SkippedRecord, SourceEntries,
};
use crate::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelId, ModelRef, ProviderDescriptor, ProviderId, SourceLayer, TransportKind,
};

fn entry(provider_id: &str, model: &str, display: &str) -> CatalogueEntry {
    CatalogueEntry {
        provider: ProviderDescriptor {
            id: ProviderId::new(provider_id).unwrap(),
            display_name: Some(provider_id.to_string()),
            transport: TransportKind::OpenAiCompletions,
            auth: AuthIdentity::ApiKey,
        },
        model: ModelDescriptor {
            reference: ModelRef::new(
                ProviderId::new(provider_id).unwrap(),
                ModelId::new(model).unwrap(),
            ),
            display_name: Some(display.to_string()),
            capabilities: ModelCapabilities {
                effort_levels: Vec::new(),
                input_modalities: vec!["text".to_string()],
                context_window: 128_000,
                max_output_tokens: 4096,
                context_window_explicit: true,
                max_output_tokens_explicit: false,
                reasoning: false,
                cost: ModelCost::default(),
            },
            availability: Availability::runnable(),
        },
    }
}

struct FakeSource {
    id: &'static str,
    result: Result<SourceEntries, String>,
}

impl CatalogueSource for FakeSource {
    fn id(&self) -> &str {
        self.id
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::BuiltIn
    }
    fn load(&self) -> Result<SourceEntries, String> {
        self.result.clone()
    }
}

struct Granting(Vec<&'static str>);

impl CredentialStatusPort for Granting {
    fn credential_available(&self, entry: &CatalogueEntry) -> bool {
        self.0.contains(&entry.reference().provider().as_str())
    }
}

struct FakeLoaded {
    source: FakeSource,
    credentials: Granting,
}

impl LoadedCatalogueInputs for FakeLoaded {
    fn sources(&self) -> Vec<&dyn CatalogueSource> {
        vec![&self.source]
    }
    fn credentials(&self) -> &dyn CredentialStatusPort {
        &self.credentials
    }
}

/// One queued load: the builtin source's result and the granted providers.
type LoadAnswer = (Result<SourceEntries, String>, Vec<&'static str>);

/// Answers each load from a queue, counting calls: the use case must load
/// on every execute (the listing stays level with on-disk edits).
struct FakeLoader {
    answers: Mutex<Vec<LoadAnswer>>,
    loads: Mutex<usize>,
}

impl FakeLoader {
    fn answering(answers: Vec<LoadAnswer>) -> Arc<Self> {
        let mut answers = answers;
        answers.reverse();
        Arc::new(Self {
            answers: Mutex::new(answers),
            loads: Mutex::new(0),
        })
    }
}

impl CatalogueInputsLoader for FakeLoader {
    fn load(&self) -> Box<dyn LoadedCatalogueInputs> {
        *self.loads.lock().unwrap() += 1;
        let (result, granted) = self
            .answers
            .lock()
            .unwrap()
            .pop()
            .expect("a queued answer for every load");
        Box::new(FakeLoaded {
            source: FakeSource {
                id: "builtin",
                result,
            },
            credentials: Granting(granted),
        })
    }
}

#[test]
fn lists_the_generation_it_published_with_runnable_status_per_entry() {
    let loader = FakeLoader::answering(vec![(
        Ok(vec![
            entry("openai-api", "gpt-5", "GPT"),
            entry("anthropic", "opus", "Opus"),
        ]
        .into()),
        vec!["anthropic"],
    )]);
    let store = CatalogueSnapshotStore::empty();
    let listed = match ListModels::new(loader.clone(), store.clone()).execute() {
        ModelListingOutcome::Listed(listed) => listed,
        other => panic!("expected a listing, got {other:?}"),
    };
    assert_eq!(listed.generation, 1);
    assert_eq!(store.current().generation(), 1, "the call published");
    let by_id: Vec<(String, bool)> = listed
        .models
        .iter()
        .map(|m| (m.entry.reference().qualified_id(), m.runnable))
        .collect();
    assert_eq!(
        by_id,
        vec![
            ("openai-api/gpt-5".to_string(), false),
            ("anthropic/opus".to_string(), true),
        ]
    );
    assert!(listed.rejected.is_empty());
}

#[test]
fn every_execute_reloads_inputs_so_edits_show_up() {
    let loader = FakeLoader::answering(vec![
        (Ok(vec![entry("openai-api", "gpt-5", "GPT")].into()), vec![]),
        (
            Ok(vec![
                entry("openai-api", "gpt-5", "GPT"),
                entry("openai-api", "gpt-6", "Next"),
            ]
            .into()),
            vec![],
        ),
    ]);
    let use_case = ListModels::new(loader.clone(), CatalogueSnapshotStore::empty());
    let first = use_case.execute();
    let second = use_case.execute();
    let (ModelListingOutcome::Listed(first), ModelListingOutcome::Listed(second)) = (first, second)
    else {
        panic!("both calls list");
    };
    assert_eq!(*loader.loads.lock().unwrap(), 2);
    assert_eq!((first.models.len(), second.models.len()), (1, 2));
    assert_eq!((first.generation, second.generation), (1, 2));
}

#[test]
fn a_failed_source_is_reported_not_listed_over_and_the_last_generation_stays() {
    let loader = FakeLoader::answering(vec![
        (Ok(vec![entry("openai-api", "gpt-5", "GPT")].into()), vec![]),
        (Err("models.json: expected value at line 1".into()), vec![]),
    ]);
    let store = CatalogueSnapshotStore::empty();
    let use_case = ListModels::new(loader, store.clone());
    assert!(matches!(use_case.execute(), ModelListingOutcome::Listed(_)));
    let outcome = use_case.execute();
    assert_eq!(
        outcome,
        ModelListingOutcome::SourcesUnavailable(vec![CatalogueSourceError {
            source: "builtin".into(),
            error: "models.json: expected value at line 1".into(),
        }])
    );
    assert_eq!(
        store.current().entries().len(),
        1,
        "last valid snapshot kept"
    );
}

#[test]
fn skipped_records_surface_as_diagnostics_naming_their_source() {
    let loader = FakeLoader::answering(vec![(
        Ok(SourceEntries {
            entries: vec![entry("openai-api", "gpt-5", "GPT")],
            skipped: vec![SkippedRecord {
                record: "openai-api/bad model".into(),
                error: "invalid model id".into(),
            }],
        }),
        vec![],
    )]);
    let ModelListingOutcome::Listed(listed) =
        ListModels::new(loader, CatalogueSnapshotStore::empty()).execute()
    else {
        panic!("lists");
    };
    assert_eq!(
        listed.rejected,
        vec![ListingDiagnostic {
            model: "openai-api/bad model".into(),
            reason: "builtin: invalid model id".into(),
        }]
    );
}

#[test]
fn domain_rejected_entries_surface_as_diagnostics_before_skipped_records() {
    // A zero context window fails domain validation after the source
    // mapped it, so it is rejected (not skipped) — and listed first.
    let mut zero_window = entry("openai-api", "gpt-5", "GPT");
    zero_window.model.capabilities.context_window = 0;
    let loader = FakeLoader::answering(vec![(
        Ok(SourceEntries {
            entries: vec![zero_window, entry("openai-api", "gpt-6", "Next")],
            skipped: vec![SkippedRecord {
                record: "openai-api/bad model".into(),
                error: "invalid model id".into(),
            }],
        }),
        vec![],
    )]);
    let ModelListingOutcome::Listed(listed) =
        ListModels::new(loader, CatalogueSnapshotStore::empty()).execute()
    else {
        panic!("lists");
    };
    assert_eq!(listed.rejected.len(), 2, "{:?}", listed.rejected);
    assert_eq!(listed.rejected[0].model, "openai-api/gpt-5");
    assert!(
        !listed.rejected[0].reason.is_empty(),
        "the domain's rejection reason is carried"
    );
    assert_eq!(listed.rejected[1].model, "openai-api/bad model");
}

#[test]
fn debug_does_not_expose_the_ports() {
    let loader = FakeLoader::answering(vec![]);
    assert_eq!(
        format!(
            "{:?}",
            ListModels::new(loader, CatalogueSnapshotStore::empty())
        ),
        "ListModels { .. }"
    );
}
