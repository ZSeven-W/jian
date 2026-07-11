use super::Runtime;
use crate::error::{CoreError, CoreResult};
use jian_ops_schema::PenDocument;

#[derive(Debug, Clone)]
pub struct ParkedBuild {
    pub target_page_id: String,
    pub schema: PenDocument,
    pub mutation_counter_at_build: u64,
    pub font_generation_at_build: u64,
    pub build_count: usize,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub enum SwapState {
    #[default]
    Idle,
    AwaitingIme {
        request_id: u64,
        parked: Box<ParkedBuild>,
    },
}

impl Runtime {
    pub fn input_frozen(&self) -> bool {
        matches!(self.swap_state, SwapState::AwaitingIme { .. })
    }

    pub fn abandon_variant_swap(&mut self) {
        if let SwapState::AwaitingIme { request_id, .. } = self.swap_state {
            self.ime_registry.detach(request_id);
        }
        self.swap_state = SwapState::Idle;
    }

    pub fn mutation_counter(&self) -> u64 {
        self.mutation_counter.get()
    }

    pub fn switch_variant(&mut self, target_page_id: &str) -> CoreResult<bool> {
        if self.active_variant_page_id.as_deref() == Some(target_page_id) {
            return Ok(false);
        }
        let source = self
            .variant_source
            .as_ref()
            .ok_or_else(|| CoreError::Layout("variant source is not configured".into()))?;
        let page = source
            .pages
            .as_ref()
            .and_then(|pages| pages.iter().find(|page| page.id == target_page_id))
            .cloned()
            .ok_or_else(|| CoreError::Layout(format!("unknown variant page `{target_page_id}`")))?;
        let mut schema = source.clone();
        schema.pages = Some(vec![page]);
        let parked = ParkedBuild {
            target_page_id: target_page_id.to_owned(),
            schema,
            mutation_counter_at_build: self.mutation_counter(),
            font_generation_at_build: 0,
            build_count: 1,
            started_at_ms: self.now_ms,
        };
        if let SwapState::AwaitingIme {
            parked: current, ..
        } = &mut self.swap_state
        {
            let started_at_ms = current.started_at_ms;
            let build_count = current.build_count + 1;
            **current = ParkedBuild {
                started_at_ms,
                build_count,
                ..parked
            };
            return Ok(false);
        }
        if let Some(snapshot) = self.active_ime_snapshot() {
            let request_id = self.begin_ime_handshake(snapshot);
            self.swap_state = SwapState::AwaitingIme {
                request_id,
                parked: Box::new(parked),
            };
            return Ok(false);
        }
        self.commit_parked(parked)?;
        Ok(true)
    }

    pub(crate) fn commit_parked(&mut self, mut parked: ParkedBuild) -> CoreResult<()> {
        if parked.mutation_counter_at_build != self.mutation_counter() {
            parked.mutation_counter_at_build = self.mutation_counter();
            parked.build_count += 1;
        }
        let target = parked.target_page_id.clone();
        let variant_source = self.variant_source.clone();
        let variant_table = self.variant_table.clone();
        let active_screen_path = self.active_screen_path.clone();
        self.replace_document(parked.schema)?;
        self.variant_source = variant_source;
        self.variant_table = variant_table;
        self.active_screen_path = active_screen_path;
        self.active_variant_page_id = Some(target.clone());
        self.active_page_key = target.clone();
        self.widget_states.set_page_key(target);
        self.gestures.reset();
        self.focus.clear();
        self.swap_state = SwapState::Idle;
        self.mutation_counter.set(self.mutation_counter.get() + 1);
        Ok(())
    }

    pub(crate) fn complete_parked_after_ime(&mut self, request_id: u64) {
        let state = std::mem::take(&mut self.swap_state);
        match state {
            SwapState::AwaitingIme {
                request_id: current,
                parked,
            } if current == request_id => {
                if let Err(error) = self.commit_parked(*parked) {
                    self.load_warnings
                        .push(format!("variant swap commit failed: {error}"));
                    self.swap_state = SwapState::Idle;
                }
            }
            other => self.swap_state = other,
        }
    }
}
