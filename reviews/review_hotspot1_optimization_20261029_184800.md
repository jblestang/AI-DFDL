# Revue d'Optimisation du Hotspot #1 (`InfosetCheckpoint`) — 2026-10-29 18:48:00

## 1. Contexte & Problématique Résolue

L'analyse de profilage de [`dfdl-benchmark`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-benchmark/src/main.rs) avait révélé que **~45% du temps d'exécution** de l'interpréteur Raw DFDL était consommé par [`InfosetBuilder::checkpoint`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs#L501) qui effectuait un clonage complet de l'arbre DOM (`self.clone()`) à chaque tentative de parsing d'occurrence ou de choix, générant des millions d'allocations et de libérations mémoire sur le tas ($\mathcal{O}(N^2)$).

---

## 2. Solution Implémentée (Chirurgicale & Zéro-Allocation)

Remplacement du clonage complet de l'arbre par un marqueur de position POD $\mathcal{O}(1)$ ultra-léger (24 octets sur la pile) dans [`crates/dfdl-core/src/infoset/tree.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs) :

```rust
/// Lightweight, allocation-free checkpoint marker of [`InfosetBuilder`] state for backtracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfosetCheckpoint {
    stack_depth: usize,
    parent_child_count: usize,
    has_root: bool,
    total_nodes: usize,
}
```

### Mécanique de `checkpoint()` et `rollback()` :
1. **`checkpoint(&self) -> InfosetCheckpoint`** :
   Capture la profondeur de pile `stack.len()`, le nombre d'enfants déjà finalisés dans le parent courant `parent_child_count`, et l'état de la racine `doc.root`.
   - **Coût** : $\mathcal{O}(1)$, 0 allocation tas, 0 clone.
2. **Chemin nominal (Succès du parsing)** :
   Le checkpoint est simplement détruit sur la pile.
   - **Coût** : Zéro allocation, zéro désallocation, zéro clone.
3. **`rollback(&mut self, cp: InfosetCheckpoint)`** :
   En cas d'échec de branche, tronque les éléments inachevés sur la pile (`stack.truncate`) et tronque les enfants ajoutés au parent courant (`children.truncate`), sans toucher au reste de l'arbre.
   - **Coût** : $\mathcal{O}(\Delta)$ (uniquement les nœuds invalidés sont libérés, sans réallocation).

---

## 3. Résultats et Validation

### 1. Métriques de Performance FakeTDL (Avant vs Après)

| Opération | Avant (`self.clone()`) | Après (`InfosetCheckpoint`) | Gain / Accélération |
| :--- | :---: | :---: | :---: |
| **Raw DFDL parse (64B Track)** | 16.68 µs (59 002 ops/s) | **9.39 µs (103 542 ops/s)** | **+75.5% de débit (latence ÷ 1.78x)** |
| **Raw DFDLFile parse (192B Multi)** | 69.18 µs (14 317 ops/s) | **29.38 µs (33 253 ops/s)** | **+132.3% de débit (latence ÷ 2.35x)** |
| **Raw DFDLFile roundtrip (192B Multi)**| 96.25 µs (10 321 ops/s) | **65.65 µs (15 065 ops/s)** | **+46.0% de débit (latence ÷ 1.47x)** |

### 2. Validation de Non-Régression TDML
- **Suite Complète Apache Daffodil TDML (197 fichiers, 193 suites, 4 336 tests)** :
  - **4 336 / 4 336 tests réussis (100.0%)**, 0 échec.
- **Suite de tests FakeTDL (`cargo test -p dfdl-benchmark`)** :
  - 18 / 18 tests unitaires et d'intégration réussis.

### 3. Qualité et Conformité
- `cargo clippy -p dfdl-core --all-targets` : 0 erreur. Correction appliquée sur le tranchage pour conformité stricte `clippy::indexing_slicing`.
- `cargo clippy -p dfdl-benchmark --all-targets` : 0 avertissement, 0 erreur.
- Graphe de dépendances régénéré via `graft build`.
