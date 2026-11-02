# Revue d'Expertise Génie Logiciel & Architecture : `dfdl-core`

- **Date :** 2026-10-28
- **Horodatage :** 17:47:26 UTC+1
- **Cible :** `crates/dfdl-core` (Architecture du Moteur & Qualité Logicielle)
- **Auteur :** Expert Architecture Logicielle & Qualité de Code

---

## 1. Cartographie Architecturale & Organisation Modulaire

Le crate `dfdl-core` est conçu selon une architecture multicouche avec séparation stricte des responsabilités (Separation of Concerns) :

```
                  ┌─────────────────────────────────────────┐
                  │          API Utilisateur DFDL           │
                  └────────────────────┬────────────────────┘
                                       │
         ┌─────────────────────────────┴─────────────────────────────┐
         ▼                                                           ▼
┌──────────────────┐                                       ┌──────────────────┐
│   ParserEngine   │ (Streaming Parse)                     │  UnparserEngine  │ (Streaming Unparse)
└────────┬─────────┘                                       └────────┬─────────┘
         │                                                          │
         ├─────────────────────────────┬────────────────────────────┤
         ▼                             ▼                            ▼
┌──────────────────┐          ┌──────────────────┐        ┌──────────────────┐
│  BitReader / IO  │          │  CompiledSchema  │        │   XPath Engine   │
│  (Zero-Copy I/O) │          │  (Immutable IR)  │        │  (expr/eval.rs)  │
└──────────────────┘          └──────────────────┘        └──────────────────┘
         ▲                                                          │
         └─────────────────────────────┬────────────────────────────┘
                                       ▼
                              ┌──────────────────┐
                              │    Infoset       │
                              │  (Tree / Events) │
                              └──────────────────┘
```

### Évaluation des Couplages :
- **Points forts :**
  - **Invariance de `CompiledSchema` :** L'IR de schéma est complètement immuable une fois compilé. Il implémente `Send + Sync`, permettant à plusieurs cœurs CPU ou threads asynchrones de parser des flux concurrents en partageant une instance unique de schéma en mémoire (zéro copie de schéma).
  - **Indépendance de la couche I/O :** Les traits [`ByteSource`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/io/traits.rs) et [`ByteSink`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/io/traits.rs) abstraient la provenance des octets (tampon mémoire, tranche fixe, ou futur socket réseau).

---

## 2. Analyse de la Dette Technique & Complexité Structurelle

### 2.1. Les Monolithes `eval.rs` et `unparser.rs`
- **Taille critique :**
  - [`src/expr/eval.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval.rs) : **345 Ko**, plus de 7 500 lignes.
  - [`src/kernel/unparser.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser.rs) : **300 Ko**, plus de 6 500 lignes.
- **Diagnostic de maintenabilité :**
  - Dans `eval.rs`, l'interprétation des expressions combine le parcours d'arbre AST Pest, les règles de coercition de types XPath, l'évaluation de chemins relatifs (`../sibling`), et l'implémentation de plus de 40 fonctions standard.
  - Dans `unparser.rs`, la sérialisation des éléments, la gestion des délimiteurs, l'alignement, le padding et la résolution des suspensions sont condensés dans un unique fichier.
- **Plan de Refactorisation Recommandé :**
  1. **Scission de `eval.rs` :**
     - `expr/eval/context.rs` : Environnement d'évaluation, portées de variables et pile d'infoset.
     - `expr/eval/functions_core.rs` : Fonctions XPath standard (`fn:concat`, `fn:substring`, `fn:abs`, etc.).
     - `expr/eval/functions_dfdl.rs` : Fonctions DFDL spécifiques (`dfdl:occursIndex`, `dfdl:checkConstraints`, etc.).
     - `expr/eval/navigation.rs` : Résolution des axes et chemins d'accès XPath (`/`, `../`, etc.).
  2. **Scission de `unparser.rs` :**
     - `kernel/unparser/element.rs` : Sérialisation des éléments simples et complexes.
     - `kernel/unparser/suspensions.rs` : Machine à états des suspensions et résolution différée.
     - `kernel/unparser/framing.rs` : Gestion des délimiteurs, séparateurs et alignements physiques.

### 2.2. Dualité DOM vs Streaming dans l'Infoset
- **Constat d'architecture :**
  - Le parseur produit par défaut un [`InfosetDocument`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs) (arbre DOM complet en mémoire vive).
  - Bien que le trait [`InfosetEventSink`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/events.rs) soit défini, la plupart des chemins de haut niveau et des benchmarks instancient l'arbre DOM complet.
  - Pour des fichiers volumineux (captures PCAP réseau de plusieurs Go ou flux radar continus), instancier un nœud DOM par paquet provoque une saturation du cache et une pression excessive sur le gestionnaire de mémoire.
- **Recommandation :**
  - Promouvoir l'API de streaming événementiel direct : permettre de streamer les événements (`StartElement`, `SimpleValue`, `EndElement`) directement vers l'application consommatrice (ex: Wireshark, extracteur de métriques) sans jamais construire l'arbre DOM complet.

---

## 3. Robustesse, Sécurité & Déterminisme

### 3.1. Mécanisme de Budget de Travail (`WorkBudget`)
- Dans [`src/limits.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/limits.rs), chaque instance de moteur reçoit un compteur `WorkBudget` qui décrémente à chaque élément parsé et chaque étape de rétro-poursuite.
- En cas de grammaire DFDL cyclique ou de données conçues pour provoquer un backtracking exponentiel (ReDoS ou "Zip Bomb"), le moteur interrompt immédiatement le traitement avec une erreur de budget de travail.
- Cette conception est essentielle pour les applications critiques et les environnements de télécommunications exposés à des données non fiables.

### 3.2. Qualité des Tests et Vérification
- **Résultats des tests unitaires :** **295 tests unitaires réussis** en 0.02s.
- Les tests couvrent rigoureusement :
  - Les calculs d'alignements bit par bit.
  - Les codecs de décimaux compactés (BCD, IBM Comp-3, IBM 4690).
  - Les règles d'évaluation d'expressions XPath et les arrondis conformes IEEE 754.
  - La validation des facettes et des types primitifs.

---

## 4. Tableau Synthétique des Recommandations Logicielles

| Domaine | Problématique | Criticité | Action Recommandée |
|:---|:---|:---:|:---|
| **Maintenabilité** | `eval.rs` (345 Ko) et `unparser.rs` (300 Ko) sont trop volumineux. | Élevée | Découper en sous-modules thématiques. |
| **Performance** | Construction systématique de l'arbre DOM complet. | Moyenne | Généraliser l'usage du mode streaming (`InfosetEventSink`). |
| **Optimisation** | Parsing Pest dynamique des expressions XPath en phase de parse. | Moyenne | Précompiler les expressions en bytecode IR ou opcodes VM. |
| **Traçabilité** | Diagnostic d'interblocage lors de suspensions cycliques. | Faible | Formaliser une erreur explicite de type `UnparseDeadlockError`. |

---

## 5. Conclusion Générale Logiciel
`dfdl-core` est une base logicielle d'une robustesse et d'une performance exceptionnelles. L'isolation stricte `#![no_std]`, le modèle de données immuable de `CompiledSchema`, l'utilisation de `WorkBudget` et le zéro-panique en production en font une infrastructure de confiance pour le traitement de données critiques. Le découpage modulaire des deux fichiers monolithiques (`eval.rs` et `unparser.rs`) parachèvera son excellence architecturale.
