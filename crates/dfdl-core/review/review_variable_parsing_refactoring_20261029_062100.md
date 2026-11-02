# Revue d'expertise : Refactorisation du parsing des noms de variables dans `variables.rs`

**Date & Timestamp :** 2026-10-29T06:21:00+01:00  
**Cible :** [`crates/dfdl-core/src/expr/variables.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/variables.rs)  
**Auteur :** Antigravity / Expertise Rust, DFDL v1.0 & Génie Logiciel

---

## 1. Contexte et Problématique

Dans [`crates/dfdl-core/src/expr/variables.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/variables.rs), le motif suivant était dupliqué dans de multiples fonctions (`set_variable_validated`, `set_external_variable`, `new_variable_instance`, `pop_variable_instance`, `get_variable`, `get_variable_validated`) :

```rust
let clean = name.local_name.trim_start_matches('$');
let local = clean.split(':').next_back().unwrap_or(clean);
if let Some((var_idx, var)) = self.variables.iter_mut().enumerate().find(|(_, v)| {
    let v_local = v.name.local_name.split(':').next_back().unwrap_or(&v.name.local_name);
    v_local == local || v.name.local_name == clean || v.name.local_name == name.local_name
})
```

### Problèmes identifiés :
1. **Duplication de code (Violation DRY) :** Le découpage du préfixe optionnel, du sigil `$` et de la partie locale était répété plus de 6 fois avec de légères variations incohérentes (`v.name.local_name == local` vs `v.name.local_name == name.local_name`).
2. **Fragilité de la résolution de noms DFDL :** Si une variable est définie avec préfixe explicite (`ex:counter`), un accès avec un autre préfixe (`other:counter`) pouvait correspondre par inadvertance si seule la partie locale était comparée sans vérifier la cohérence du préfixe.
3. **Complexité d'emprunt (`borrow checker`) :** `VariableMap` gère des mutations concurrentes (`self.undo_log`, `self.variables`, `self.escalate_warnings`). Les itérations directes ou `iter_mut()` bloquaient l'accès aux autres champs de la structure.

---

## 2. Refactorisation Réalisée

### A. Fonction utilitaire d'extraction `parse_variable_name`
```rust
#[inline]
#[must_use]
pub fn parse_variable_name(raw: &str) -> (Option<&str>, &str)
```
- Nettoie le sigil initial `$` sans allocation mémoire (`trim_start_matches('$')`).
- Découpe en `(prefix, local)` en O(1) sur des tranches de chaîne (`&str`).
- Ratio de documentation conforme (docstrings détaillés et exemples d'utilisation).

### B. Méthodes sur `DfdlVariable`
- `pub fn local_name(&self) -> &str` : Retourne la partie locale pure du QName de la variable.
- `pub fn matches_name(&self, raw: &str) -> bool` : Compare un nom brut (`$prefix:local`, `prefix:local`, `$local`, `local`) avec le `QName` canonique de la variable, en validant à la fois le préfixe (si fourni) et le nom local.

### C. Méthodes de recherche sur `VariableMap`
- `pub fn find_var(&self, name: &str) -> Option<(usize, &DfdlVariable)>`
- `pub fn find_var_mut(&mut self, name: &str) -> Option<(usize, &mut DfdlVariable)>`
- `pub fn find_var_index(&self, name: &str) -> Option<usize>` : Permet de découpler la recherche d'index de l'emprunt mutable de `self.variables` et `self.undo_log`.

### D. Adaptation sécurisée sans panique (`-D clippy::indexing-slicing`)
- Utilisation de `self.variables.get_mut(var_idx)` et `self.variables.get(var_idx)` au lieu de l'indexation directe `self.variables[var_idx]`.

---

## 3. Matrice de Validation

| Vérification | Commande | Résultat |
| :--- | :--- | :--- |
| **Tests unitaires `dfdl-core`** | `cargo test -p dfdl-core` | **297 passés, 0 échecs** |
| **Doc-tests `dfdl-core`** | `cargo test -p dfdl-core --doc` | **4 passés, 0 échecs** |
| **Analyse statique et linter** | `cargo clippy --workspace --all-targets -- -D warnings` | **0 avertissement, 0 erreur** |
| **Tests des schémas DFDL** | `cargo test -p dfdl-tests --lib -- test_dfdlschema_` | **15 suites exécutées avec succès** |
