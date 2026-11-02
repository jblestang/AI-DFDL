# Revue d'Expertise Rust : `dfdl-core`

- **Date :** 2026-10-28
- **Horodatage :** 17:47:26 UTC+1
- **Cible :** `crates/dfdl-core` (`src/io/`, `src/infoset/`, `src/schema/`, `src/expr/`, `src/kernel/`, etc.)
- **Auteur :** Expert Rust / Systèmes Temps Réel & Performance
- **Statut de compilation :** Passé (0 warning, 0 clippy warning, `#![no_std]` + `alloc`)

---

## 1. Vue d'Ensemble & Périmètre Technique

Le crate `dfdl-core` constitue le cœur névralgique de l'implémentation DFDL. Il encapsule l'ensemble du moteur d'exécution bas niveau : flux binaire au bit près (`BitReader`, `BitWriter`), structures de données d'infoset, représentation intermédiaire (`CompiledSchema`), moteur d'évaluation des expressions XPath 2.0 et moteurs de parsing et d'unparsing.

### Modules audités :
- [`src/io/`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/io) : `bitstream.rs`, `source.rs`, `sink.rs`, `traits.rs`.
- [`src/infoset/`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset) : `tree.rs`, `value.rs`, `events.rs`, `state.rs`.
- [`src/schema/`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema) : `ir.rs`, `builder.rs`.
- [`src/expr/`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr) : `eval.rs`, `parser.rs`, `properties.rs`, `variables.rs`, `ast.rs`.
- [`src/kernel/`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel) : `mod.rs`, `layer.rs`, `unparser.rs`, `parser/`.
- Utilitaires : `encoding.rs`, `error.rs`, `limits.rs`, `pattern.rs`, `types.rs`, `util.rs`.

---

## 2. Analyse Approfondie Idiomes Rust, Sécurité & Performance

### 2.1. Respect strict du `#![no_std]` et Intégrité Mémoire
- **Architecture de compilation :**
  - Crate intégralement conçu sous `#![no_std]` avec `extern crate alloc;`.
  - Zéro dépendance vers le runtime OS standard (pas de `std::fs`, `std::net`, `std::thread`, `std::time`).
  - Toutes les opérations calendaires et arithmétiques sont autonomes.
- **Politique de panique et résilience :**
  - **Zéro `unwrap()` / `expect()` en production :** L'intégralité du chemin critique retourne `DFDLResult<T>`.
  - Protection systématique contre les dépassements d'entiers via [`src/util.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/util.rs) :
    - `checked_add_usize`, `checked_mul_usize`, `checked_sub_usize`.
    - `get_checked(slice, index)` pour les accès indexés sécurisés.
    - `try_push(vec, item)` pour contrôler les allocations et prévenir les paniques OOM.

### 2.2. Manipulation Binaire de Haute Précision (`io/bitstream.rs`)
- **Lecture et Écriture au niveau du bit :**
  - Prise en charge chirurgicale de `BitOrder::MostSignificantBitFirst` (MSBF) et `BitOrder::LeastSignificantBitFirst` (LSBF).
  - Gestion des alignements arbitraires (champs de 1 à 64 bits non alignés sur les octets).
- **Optimisation Zero-Copy Peeking :**
  - Intégration de `peek_bytes(len)` et `peek_slice(len)` permettant d'inspecter les délimiteurs et préfixes directement sur le tampon mémoire source (`&[u8]`) sans aucune allocation sur le tas ni copie de mémoire intermédiaire.
  - Cette optimisation a permis de diviser la latence de parsing par 2 sur les flux de télémesure FakeTDL (cf. benchmark 60 µs vs 113 µs).

### 2.3. Pression d'Allocation sur l'Arbre Infoset DOM
- **Le goulot d'étranglement de l'arbre DOM :**
  - [`src/infoset/tree.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs) modélise l'arbre de données parsées via [`InfosetElement`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs) comportant des `HashMap<String, Vec<InfosetNode>>` et des `Vec<InfosetNode>`.
  - Chaque champ simple (ex: un entier 16 bits) requiert l'allocation d'une structure `InfosetElement`, d'un `QName` (`String`), et d'un conteneur d'enfants.
  - **Recommandation Rust :**
    - Promouvoir le mode streaming événementiel via le trait [`InfosetEventSink`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/events.rs).
    - Pour les cas nécessitant un accès complet à l'arbre, basculer sur l'allocateur par arène compacte [`dfdl-vm::arena::InfosetArena`] qui stocke les nœuds dans un vecteur contigu sans pointeurs de tas individuels.

### 2.4. Monolithes de Code et Impact sur le Compilateur LLVM
- **Fichiers hors normes :**
  - [`src/kernel/unparser.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser.rs) : **300 Ko**, plus de 6 500 lignes.
  - [`src/expr/eval.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval.rs) : **345 Ko**, plus de 7 500 lignes.
  - Ces fichiers gigantesques ralentissent le parsing, le macro-expansion, la vérification de types et l'optimisation LLVM.
  - **Action recommandée :**
    1. Découper `eval.rs` en sous-modules : `eval/xpath.rs`, `eval/functions.rs`, `eval/operators.rs`, `eval/coercions.rs`.
    2. Découper `unparser.rs` en sous-modules : `unparser/element.rs`, `unparser/sequence.rs`, `unparser/suspensions.rs`, `unparser/delimiters.rs`.

---

## 3. Synthèse des Remarques & Recommandations

| ID | Sévérité | Fichier | Description | Action Recommandée |
|:---|:---:|:---|:---|:---|
| **RUST-CORE-01** | Majeur | `unparser.rs` | Fichier monolithique de 300 Ko / 6 500 lignes. | Scinder en sous-modules thématiques dans un répertoire `kernel/unparser/`. |
| **RUST-CORE-02** | Majeur | `eval.rs` | Fichier monolithique de 345 Ko / 7 500 lignes. | Scinder en sous-modules thématiques dans `expr/eval/`. |
| **RUST-CORE-03** | Moyen | `infoset/tree.rs` | Allocations multiples de `HashMap` et `String` pour chaque nœud infoset. | Généraliser le recours à `InfosetArena` ou à `InfosetEventSink`. |
| **RUST-CORE-04** | Mineur | `expr/eval.rs` | Traitement dynamique répété de chaînes d'expressions. | Compiler les expressions XPath statiques en bytecode avant exécution. |

---

## 4. Conclusion de l'Expert Rust
Le crate `dfdl-core` est une pièce d'ingénierie système remarquable. L'exécution sans panique, la gestion précise des flux binaires au bit près en `#![no_std]` et la conformité stricte aux règles de sécurité en font un moteur de niveau industriel. La restructuration modulaire des deux fichiers géants (`unparser.rs` et `eval.rs`) constitue la priorité pour pérenniser l'évolutivité du code.
