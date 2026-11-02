# Analyse de Profilage du Benchmark FakeTDL (Hotspots & Performance Gap) — 2026-10-29 14:55:00

## 1. Contexte & Méthodologie d'Analyse

À la suite de la validation intégrale de conformité TDML (100% de passage sur l'ensemble des suites et des 4 336 tests officiels Daffodil), une campagne de profilage fin a été menée sur le benchmark **FakeTDL** (`crates/dfdl-benchmark`) afin d'identifier les goulets d'étranglement CPU et mémoire au sein du moteur d'exécution.

### Conditions de mesure
- **Cible binaire** : `dfdl-benchmark` compilé avec le profil `--release` optimisé (`opt-level = 3`, LTO partiel, `debug = true` pour résolution symbolique complète des pointeurs d'instructions sans dégradation d'optimisation).
- **Plateforme matérielle** : Apple Silicon ARM64 (macOS 26.5.1 Darwin).
- **Outil d'échantillonnage** : `/usr/bin/sample` échantillonnant à 1 kHz (résolution 1 ms) sur l'ensemble des threads pendant l'exécution d'un jeu de 100 000 itérations par opération.
- **Volume de traces** : 8 340 lignes de graphe d'appel (Call Graph) et décompte d'échantillons en tête de pile (*Top of Stack*).

---

## 2. Métriques Comparatives Recueillies (100 000 itérations)

| Composant / Opération | Latence Moyenne | Latence Médiane | Débit (ops/sec) | Bande passante | Ratio vs Natif |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Raw DFDL Parse (64B Track)** | 16.68 µs | 15.17 µs | 59 002 | 3.60 MB/s | **794x plus lent** |
| **DFDL-VM Parse DOM (64B Track)** | 1.06 µs | 1.04 µs | 764 445 | 46.66 MB/s | 50.5x plus lent |
| **DFDL-VM Parse Arena (64B Track)** | 348 ns | 334 ns | 2 589 745 | 158.07 MB/s | 16.6x plus lent |
| **DFDL-VM Parse Null (64B Track)** | 242 ns | 250 ns | 3 778 034 | 230.59 MB/s | 11.5x plus lent |
| **Native Rust Decode (64B Track)** | 21 ns | 41 ns | 22 422 153 | 1 368.54 MB/s | **1.0x (Baseline)** |
| **Raw DFDL File Parse (192B Multi)** | 69.18 µs | 62.96 µs | 14 317 | 2.62 MB/s | **1 504x plus lent** |
| **DFDL-VM File Parse DOM (192B Multi)**| 2.32 µs | 2.33 µs | 339 668 | 62.20 MB/s | 50.4x plus lent |
| **DFDL-VM File Parse Arena (192B)** | 774 ns | 750 ns | 1 192 435 | 218.34 MB/s | 16.8x plus lent |
| **DFDL-VM File Parse Null (192B)** | 516 ns | 500 ns | 1 856 225 | 339.88 MB/s | 11.2x plus lent |
| **Native Rust File Decode (192B)** | 46 ns | 42 ns | 13 145 282 | 2 406.97 MB/s | **1.0x (Baseline)** |

---

## 3. Cartographie Détaillée des Hotspots (Top of Stack & Call Tree)

L'échantillonneur révèle que plus de **60% du temps total passé dans l'interpréteur Raw DFDL** ne concerne pas le décodage binaire des bits, mais la gestion dynamique de la mémoire (allocations, clones d'arbres, libérations du tas).

### Hotspot #1 (Critique) : Le Point de Restauration O(N²) de l'Arbre Infoset (`InfosetBuilder::checkpoint`)
- **Échantillons cumulés** :
  - `_xzm_free` / `_free` : **1 595 échantillons** (19.1%)
  - `_malloc_zone_malloc` / `_xzm_xzone_malloc` : **1 038 échantillons** (12.4%)
  - `Clone::clone` sur `Vec<InfosetNode>` / `InfosetElement` : **1 251 échantillons** (15.0%)
- **Source exacte** : [`crates/dfdl-core/src/infoset/tree.rs:501`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs#L501)
  ```rust
  pub fn checkpoint(&self) -> Self {
      self.clone()
  }
  ```
- **Mécanisme d'impact** :
  Lors de l'analyse d'un groupe ou d'un élément répétitif (séquences d'occurrences dans `parse_single_element_occurrence_inner` et `parse_element_with_separators`), l'interpréteur sauvegarde l'état courant de l'infoset pour pouvoir faire du backtracking en cas d'échec de branche.
  Comme `checkpoint(&self)` fait un `self.clone()`, il clone récursivement :
  - Toute la structure `InfosetDocument`
  - Tout le vecteur `stack: Vec<InfosetElement>`
  - Tous les nœuds enfants déjà analysés (`children: Vec<InfosetNode>`)
  - Toutes les chaînes de noms (`QName`) et les valeurs dynamiques `DfdlValue`.
  Pour un message contenant $N$ éléments, le moteur réalise $\sum_{k=1}^N k = \mathcal{O}(N^2)$ allocations de nœuds sur le tas. Dès que la branche réussit, ce snapshot cloné est immédiatement désalloué (d'où les millions d'appels à `free` / `_xzm_free`).

---

### Hotspot #2 (Majeur) : Allocations Récurrentes de Chaînes dans les Requêtes d'Encodage
- **Échantillons cumulés** :
  - `Pattern::is_contained_in` : **272 échantillons**
  - `String::clone` / `finish_grow` / `alloc` : **250+ échantillons**
- **Source exacte** : [`crates/dfdl-core/src/encoding.rs:456-535`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding.rs#L456-L535)
  ```rust
  pub fn encoding_char_bits(encoding: &str) -> Option<usize> {
      let enc_upper = encoding.trim().to_ascii_uppercase(); // <-- ALLOCATION TAS SUR CHAQUE CHAMP
      if enc_upper.contains("BITS") { ... }
  }

  pub fn encoding_mandatory_alignment_bits(encoding: &str) -> usize {
      let upper = encoding.trim().to_ascii_uppercase(); // <-- ALLOCATION TAS SUR CHAQUE CHAMP
      ...
  }
  ```
- **Mécanisme d'impact** :
  À chaque lecture scalaire de champ texte ou binaire, le moteur appelle `encoding_char_bits` et `encoding_mandatory_alignment_bits`. Chaque appel exécute `.to_ascii_uppercase()`, allouant une nouvelle `String` sur le tas, puis effectue une cascade de `contains(...)` avec balayage linéaire (`is_contained_in`).

---

### Hotspot #3 (Modéré) : Poussée et Dépilement de Traces de Chemin (`InfosetPath::try_push` / `pop`)
- **Échantillons cumulés** : **80 échantillons**
- **Source exacte** : [`crates/dfdl-core/src/types.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/types.rs) et [`crates/dfdl-core/src/kernel/parser/element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)
- **Mécanisme d'impact** :
  Pour alimenter le chemin d'infoset courant (notamment pour l'évaluation des expressions relatives et les messages d'erreurs contextuels), le moteur parse le segment via `PathStep::from_segment` et pousse un `PathStep` contenant une `String` sur `InfosetPath`. Même sur les messages binaires ultra-rapides où aucune expression dynamique n'est évaluée, ce suivi de chemin dynamique engendre des poussées/dépilements permanents avec allocation de vecteur.

---

### Hotspot #4 (Spécifique à l'Unparse) : Reformatage et Délimiteurs
- **Échantillons cumulés** : **~120 échantillons**
- **Fonctions impliquées** :
  - `unparse_binary_calendar` (FakeTDL horodatage DfdlValue::DateTime)
  - `write_evaluated_delimiter` et `decode_unparse_delimiter`
  - `extract_delimiter_tokens`
- **Mécanisme d'impact** :
  Sur la sérialisation, la conversion inverse des valeurs typées (notamment `DateTime` vers champs binaires DFDL) recalcule les représentations et analyse les entités délimiteurs sans mémoïsation.

---

## 4. Analyse Architectural du Goulet d'Étranglement : Pourquoi DFDL-VM est 15x à 134x Plus Rapide

La comparaison directe des architectures montre pourquoi le moteur **DFDL-VM** s'approche du natif :
1. **Élimination du Backtracking par Clone lourd** :
   Dans DFDL-VM, le flux d'instructions est compilé linéairement. Aucune copie intégrale de structure n'a lieu lors de la descente.
2. **Pré-résolution Statique des Propriétés** :
   Toutes les constantes (alignement en bits, largeur d'encodage, décalages d'octets, types scalaires) sont précalculées au moment de la compilation du schéma et encodées directement dans les opérandes des opcodes du bytecode (ex: `OpReadBits(16)`). Aucun appel à `encoding_char_bits` ou `to_ascii_uppercase` n'est réalisé au runtime.
3. **Puits d'Événements Découplé** :
   - Mode `DomBuilderSink` : génère l'arbre DOM complet en 1.06 µs (vs 16.68 µs pour l'interpréteur AST, gain de 15.7x).
   - Mode `ArenaEventSink` : stocke les événements dans une mémoire contiguë pré-réservée, supprimant toute allocation dynamique sur le chemin critique (348 ns, gain de 47.9x).
   - Mode `NullEventSink` : validation "Wire-speed" pure (242 ns, gain de 68.9x sur message track et 134.1x sur fichier multi-messages).

---

## 5. Synthèse & Pistes d'Optimisation Futures (Non appliquées conformément à la consigne)

Pour référence future (sans aucune modification du code actuel) :
1. **Remplacement du `checkpoint` par un marqueur léger** :
   Remplacer `InfosetBuilder.checkpoint()` qui clone l'arborescence entière par un simple marqueur d'indices `(stack_depth, children_count_in_top)` permettant de tronquer les enfants ajoutés en cas de rollback, sans jamais allouer ni cloner l'arbre existant.
2. **Élimination des allocations de chaînes dans `encoding.rs`** :
   Utiliser des comparaisons `eq_ignore_ascii_case` directes sur la chaîne empruntée `&str`, ou stocker l'encodage sous forme d'énumération compilée (`ResolvedEncoding`) dans les propriétés résolues pour éviter tout calcul de chaîne à l'exécution.
3. **Lazy Path Tracking** :
   Ne maintenir le vecteur `InfosetPath` que lorsque des expressions XPath relatives ou des diagnostics l'exigent, évitant les allocations systématiques sur chaque élément.
