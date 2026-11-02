# Revue d'Implémentation Génie Logiciel : Maintenabilité dans `dfdl-core`

- **Date :** 2026-10-28
- **Horodatage :** 22:08:00 UTC+1
- **Cible :** `crates/dfdl-core` (`src/expr/eval/` et `src/kernel/unparser/`)
- **Auteur :** Expert Architecture Logicielle & Qualité de Code
- **Statut :** Validé (100% de succès aux tests, zéro régression, zéro avertissement Clippy)

---

## 1. Contexte & Problématique Traitée

L'audit d'expertise logicielle initial ([`crates/dfdl-core/review/expert_logiciel.md`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/review/expert_logiciel.md#L49-L66)) a identifié deux monolithes critiques dégradant la maintenabilité et les temps de compilation :
1. [`src/expr/eval.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval.rs) : **346 Ko** et **8 974 lignes**, regroupant la navigation d'arbre AST, le contexte d'évaluation, les coercitions arithmétiques XPath, plus de 40 fonctions standard et DFDL, et une imposante suite de tests unitaires.
2. [`src/kernel/unparser.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser.rs) : **300 Ko** et **6 247 lignes**, condensant la sérialisation des éléments simples et complexes, le framing binaire/texte, les délimiteurs, les alignements, le padding, les suspensions/variables, et les tests unitaires.

---

## 2. Restructuration Modulaire Appliquée

### 2.1. Découpage du Moteur d'Évaluation XPath (`crates/dfdl-core/src/expr/eval/`)

Le fichier monolithique `eval.rs` a été supprimé et remplacé par un module partitionné selon les responsabilités techniques :

| Fichier | Lignes | Rôle & Responsabilité |
|:---|:---:|:---|
| [`context.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/context.rs) | 193 | Constante `MAX_EVAL_DEPTH` et structure `ExprContext` avec gestion du budget de travail et résolutions sécurisées. |
| [`navigation.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/navigation.rs) | 685 | Résolution des axes XPath (`/`, `../`, `.`), normalisation des chemins (`normalize_infoset_path`), calcul de longueur (`calc_elem_value_length`), et `resolve_path`. |
| [`operators.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/operators.rs) | 617 | Évaluation typée des opérateurs unaires et binaires (`eval_unary`, `eval_binary`, `eval_add`, `eval_sub`, `eval_mul`, `eval_div`, `eval_idiv`, `eval_mod`, `eval_cmp`, `coerce_numeric`). |
| [`helpers.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/helpers.rs) | 391 | Coercitions scalaires (`value_to_i64`, `value_to_f64`, `value_to_bool`), opérations bit à bit, décomposition calendaire (`parse_date_components`, etc.), et fonctions d'arrondi XPath 2.0 (`xpath_round_*`, `xpath_floor_*`, `xpath_ceil_*`). |
| [`functions.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/functions.rs) | 1 198 | Évaluation unifiée des fonctions standard XPath et des extensions DFDL (`eval_fn_call`). |
| [`mod.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/mod.rs) | 517 | Coordinateur principal (`eval_expr`, `eval_prop_str_with_ctx`, `lookup_variable`) et réexports publics (`pub use context::{ExprContext, MAX_EVAL_DEPTH}; pub use navigation::resolve_path;`). |
| [`tests.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/tests.rs) | 5 450 | Suite exhaustive des tests unitaires d'évaluation. |

### 2.2. Découpage du Moteur d'Unparsing (`crates/dfdl-core/src/kernel/unparser/`)

Le fichier monolithique `unparser.rs` a été supprimé et remplacé par des sous-modules spécialisés tirant parti des blocs d'implémentation séparés `impl<'a, S: ByteSink> UnparserEngine<'a, S>` :

| Fichier | Lignes | Rôle & Responsabilité |
|:---|:---:|:---|
| [`framing.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/framing.rs) | 453 | Gestion physique des octets de remplissage (`fill_byte_value`, `write_fill_padding`), décodage des entités de délimiteurs (`decode_unparse_delimiter`), résolution des préfixes de longueur (`write_prefix_length`), schémas d'échappement (`apply_escape_scheme`), et injection des délimiteurs/séparateurs. |
| [`suspensions.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/suspensions.rs) | 129 | Résolution dynamique des contextes d'expressions (`make_expr_context`), exécution des portées de variables DFDL (`execute_set_variables`, `execute_new_variable_instances`, `pop_new_variable_instances`), et évaluation des propriétés DFDL à l'exécution. |
| [`element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/element.rs) | 1 988 | Sérialisation des types simples (textuels, binaires, décimaux compactés BCD / IBM 4690, calendaires, booléens), validation de non-troncature et d'arrondis (`validate_text_number_rounding`, `validate_unparse_value`), remappage PUA (`remap_pua_to_raw_chars`), et mesure de taille de contenu (`measure_element_content_bits`). |
| [`mod.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/mod.rs) | 1 643 | Définition de la structure `UnparserEngine` avec champs `pub(crate)`, cycle de vie (`new`, gestion des modes de validation, collecte d'erreurs), dispatching récursif des termes et branches de choix (`unparse_document`, `unparse_term`, `unparse_element`). |
| [`tests.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/tests.rs) | 2 078 | Suite exhaustive des tests unitaires de l'unparser. |

---

## 3. Résultats des Vérifications Logicielles

Toutes les vérifications de conformité et de non-régression ont été exécutées avec succès :

1. **Compilation stricte sans avertissements :**
   ```bash
   cargo check --workspace --all-targets
   # Status: OK, 0 avertissements
   ```
2. **Vérification Clippy intégrale :**
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   # Status: OK, 0 erreurs, 0 avertissements
   ```
3. **Tests unitaires de `dfdl-core` :**
   ```bash
   cargo test -p dfdl-core
   # Résultat : 295/295 tests réussis en 0.02s (+ 4 doc-tests réussis)
   ```
4. **Tests d'intégration `dfdl-vm` et `dfdl-example` :**
   ```bash
   cargo test -p dfdl-vm -p dfdl-example
   # Résultat : 36/36 tests réussis (dont extraction PCAP et conformité ASTERIX)
   ```
5. **Suite officielle de conformité TDML (Apache Daffodil) :**
   ```bash
   cargo test -p dfdl-tests -- test_apache_daffodil_official_tdml_suite
   # Résultat : 1591/1591 tests réussis (100.0% de conformité officielle)
   ```
6. **Mise à jour du graphe d'architecture Graft :**
   ```bash
   graft build
   # Résultat : 2016 nœuds, 3885 arêtes synchronisés
   ```

---

## 4. Conclusion

La recommandation **Maintenabilité** de la revue d'expertise logicielle pour `dfdl-core` est intégralement prise en compte et validée. Les deux géants monolithiques (`eval.rs` et `unparser.rs`) ont cédé la place à une arborescence modulaire hautement maintenable, respectant rigoureusement `#![no_std]`, avec une API publique 100% rétrocompatible.
