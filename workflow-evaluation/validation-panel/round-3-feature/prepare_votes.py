#!/usr/bin/env python3
"""Preregister overlap resolution and equal independent voting prompts after proposals lock."""
import json
from pathlib import Path
p=Path(__file__).resolve().parent;a=json.loads((p/'alternatives.json').read_text());ids=[x['alternative_id'] for x in a['alternatives']]
rule={'eligibility':'>=2 YES and no unresolved goal conflict','overlap_selection':'Among eligible alternatives for the same field: majority pairwise preference wins; if no unique Condorcet winner, lowest sum of ranks wins; if tied, fewer UTF-8 bytes in new guidance wins; if still tied, lexicographically smallest alternative ID. Status quo may be ranked but cannot invalidate an otherwise >=2-YES alternative. Substantive goal conflicts or cross-step dependencies are escalated to parent, not silently resolved.','frozen_before_votes':True,'no_synthesis':True};(p/'vote-rule.json').write_text(json.dumps(rule,indent=2)+'\n')
for j in range(1,4):
 example={'judge_id':f'judge-{j}','phase':'feature-round-3-votes','votes':[{'alternative_id':aid,'vote':'YES|NO|ABSTAIN','rationale':'','goal_conflicts':[]} for aid in ids],'rankings':[{'group_id':g['group_id'],'order':g['alternatives']+['STATUS_QUO'],'rationale':''} for g in a['overlap_groups']],'binding_selection_confound':'','cross_panel_anchor_variability':'','residual_risks':[]}
 prompt=f'''You are independent vote judge-{j}. All Feature proposals are sealed. Read {p}/alternatives.json, {p}/vote-rule.json and assigned-template-reference.json. Use your retained Feature evidence. Do not read other votes, attribution-private files, or other ballots. No rescore or new proposals.

Vote YES/NO/ABSTAIN on EVERY exact alternative: {', '.join(ids)}. YES means acceptable as a standalone exact replacement, not that overlapping texts should all apply. Rank all alternatives in each overlap group plus STATUS_QUO, most preferred first, no ties. Never merge wording. Evaluate minimality, generic transfer, meaningful chronological reproduction before fix, truthful late-entry reporting, evidence-first non-executable alternatives, no fake failure, and preservation of unrelated work. No language/tool/host mandate or benchmark task coaching. At least two YES and no unresolved goal conflict are necessary; the supplied overlap rule chooses one eligible exact replacement per field. Identify dependencies or conflicts in residual risks.

Existing test_design guidance already asks for pre-implementation verification. Evaluate early intake salience without a circular dependency or duplicated check requirement; focus template-only, no task-prompt coaching. Binding-versus-selection, initial guidance salience, and reminder mechanics may explain late engagement; replacement is an unvalidated hypothesis, not proven causal improvement. Preserve cross-panel B1/B2 anchor variability; do not rescore or tailor to threshold. No source/candidate/rubric changes.

Return FINAL JSON only, filling real votes/ranks into this shape (example order is NOT a recommendation):
{json.dumps(example)}

Stop after your sealed independent vote, no writes. No other votes will be supplied before lock.
'''
 (p/f'prompts/vote-judge-{j}.txt').write_text(prompt)
m=json.loads((p/'manifest.json').read_text());m['phase']='INDEPENDENT_VOTES';m['all_proposals_locked']=True;(p/'manifest.json').write_text(json.dumps(m,indent=2)+'\n');print('Prepared',len(ids),'alternatives for independent votes')
