# Audit de Conformité et d'Exactitude des Tests DFDL v1.0

- **Date :** 2026-10-28
- **Horodatage :** 18:24:16 UTC+1
- **Cible :** `crates/dfdl-tests` (`tdml.rs`, `lib.rs`), `crates/dfdl-schema/src/compiler/tests.rs`, `crates/dfdl-core/src/kernel/parser/tests.rs`
- **Référence Standard :** OGF GFD-P-R.207 DFDL v1.0 (§3.2, §9.1, §9.2, §14, Appendix F) & Apache Daffodil TDML Specification
- **Auteurs :** Collège d'Experts (Expert DFDL, Expert Rust, Expert Assurance Qualité Logicielle)

---

## 1. Contexte & Objectif de l'Audit

Cet audit a pour but d'analyser en profondeur **l'exactitude et la fidélité de ce qui est réellement vérifié** par les tests de conformité au standard DFDL v1.0 dans le projet :
1. Les tests d'erreur vérifient-ils que l'erreur levée correspond **exactement** à la cause attendue par la spécification (message d'erreur, sous-chaîne, contexte) ?
2. Les tests distinguent-ils rigoureusement les différentes natures d'erreurs normatives :
   - **Schema Definition Error (SDE)** : Erreur statique de définition du schéma (§3.2).
   - **Processing Error (PE)** : Erreur dynamique d'analyse (parse) ou de génération (unparse) (§9.1).
   - **Validation Error (VE)** : Erreur de conformité aux facettes XML Schema (§21).
3. Existe-t-il des **"faux positifs"** (tests déclarés réussis alors que le moteur a échoué pour une mauvaise raison) dans le harnais TDML officiel ou dans les tests unitaires ?

---

## 2. Découvertes Critiques dans le Harnais de Test TDML (`crates/dfdl-tests/src/tdml.rs`)

Le harnais TDML (`TdmlRunner`) exécute les suites de tests officielles Apache Daffodil. L'audit du code source de [`crates/dfdl-tests/src/tdml.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/tdml.rs) a révélé **trois anomalies majeures de vérification ("Soft-Pass Flaws")** qui masquent des écarts réels et créent des faux positifs.

---

### 🚨 Anomalie 1 : Bypass Inconditionnel du Message d'Erreur lors du Parsing (Ligne 1409)

Dans la méthode `run_suite_with_base_dir` de [`tdml.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/tdml.rs#L1404-L1417) :

```rust
1404: match parse_res {
1405:     Ok(_) => {
1406:         report.add_failure(format!(
1407:             "Test '{}' expected parse error but succeeded. Expected: {:?}",
1408:             tc.name, tc.expected_errors
1409:         ));
1410:     }
1411:     Err(e) => {
1412:         let err_msg = format!("{:?}", e);
1413:         let matches_expected = tc.expected_errors.iter().any(|exp| {
1414:             err_msg.contains(exp) || e.message.to_string().contains(exp)
1415:         });
1416:         if matches_expected || !tc.expected_errors.is_empty() {
1417:             report.passed = report.passed.saturating_add(1);
1418:         } else {
1419:             report.add_failure(format!(
1420:                 "Test '{}' error message mismatch: got '{}'",
1421:                 tc.name, err_msg
1422:             ));
1423:         }
1424:     }
1425: }
```

#### Diagnostic d'Impact :
- **Constat :** À la ligne 1416, la condition `if matches_expected || !tc.expected_errors.is_empty()` est **toujours vraie** dès lors que le cas de test attend une erreur (`!tc.expected_errors.is_empty()`).
- **Conséquence :** Si un test TDML attend l'erreur `"Delimiter not found"`, mais que le moteur échoue pour une raison totalement différente (ex: `WorkBudgetExhausted`, `Unexpected EOF`, ou un dépassement d'indice), le test est **comptabilisé comme un SUCCÈS** !
- **Code mort :** La branche `else` (ligne 1418) ne peut **jamais** être atteinte pour un test de type parser attendant des erreurs.

---

### 🚨 Anomalie 2 : Absence de Vérification du Message sur Échec de Compilation de Schéma (Ligne 1220)

Dans [`tdml.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/tdml.rs#L1219-L1228) :

```rust
1219: let schema = match schema_res {
1220:     Ok(s) => s,
1221:     Err(e) => {
1222:         if !tc.expected_errors.is_empty() {
1223:             report.passed = report.passed.saturating_add(1);
1224:         } else {
1225:             report.add_failure(format!(
1226:                 "Test '{}' failed schema compilation: {:?}",
1227:                 tc.name, e
1228:             ));
1229:         }
1230:         continue;
1231:     }
1232: };
```

#### Diagnostic d'Impact :
- **Absence de comparaison :** Lorsqu'un schéma ne compile pas (`schema_res = Err(e)`), le runner incrémente `report.passed` immédiatement sans **jamais** vérifier si le message d'erreur `e` contient les sous-chaînes déclarées dans `<tdml:errors><tdml:error>...</tdml:error></tdml:errors>`.
- **Confusion SDE vs PE :** Si un cas de test TDML est un test d'exécution (parse) attendant une erreur d'analyse à l'exécution (`ProcessingError`), mais que le schéma échoue prématurément à la compilation (`SchemaDefinitionError`), le test est **validé à tort** alors que le parseur n'a même pas été exécuté !

---

### 🚨 Anomalie 3 : Absence de Vérification du Message lors de l'Unparsing (Ligne 1542)

Dans [`tdml.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/tdml.rs#L1542-L1544) :

```rust
1541: } else if unparse_res.is_err() {
1542:     report.passed = report.passed.saturating_add(1);
1543: }
```

#### Diagnostic d'Impact :
- Tout échec de l'unparser est accepté sans aucune vérification de conformité du message par rapport à `tc.expected_errors`.

---

## 3. Analyse des Tests Unitaires du Compilateur (`crates/dfdl-schema/src/compiler/tests.rs`)

L'audit des 114 tests unitaires de `dfdl-schema` montre à l'inverse une **très haute rigueur de validation** :
- **35 tests d'erreurs négatifs répertoriés.**
- **100% de ces 35 tests** vérifient explicitement le contenu textuel du message d'erreur retourné (`assert!(err.contains(...))`).
- Plusieurs tests vérifient en sus la classification typée : `assert_eq!(err.kind, DFDLErrorKind::SchemaDefinition)`.

### Exemples d'Exactitude Exemplaire dans `dfdl-schema` :

```rust
// Test DFDL §14.3.1 : Incompatibilité assert / discriminator
assert!(res.is_err());
let err = alloc::format!("{}", res.unwrap_err());
assert!(err.contains("both"));
assert!(err.contains("discriminator"));
assert!(err.contains("assert"));

// Test DFDL §12.3 : Propriétés obligatoires manquantes
assert!(res.is_err());
let err = alloc::format!("{}", res.unwrap_err());
assert!(err.contains("Property textStandardDecimalSeparator is not defined"));

// Test DFDL §15 : Règle Unique Particle Attribution (UPA)
assert!(res.is_err());
let err = alloc::format!("{}", res.unwrap_err());
assert!(err.contains("Unique Particle Attribution"));
```

---

## 4. Analyse des Tests d'Intégration Directs (`crates/dfdl-tests/src/lib.rs`)

Dans [`crates/dfdl-tests/src/lib.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/lib.rs), les tests de validation des sections normatives DFDL (`test_section_a` à `test_section_r`) présentent une rigueur inégale :

### 4.1. Tests Rigoureux avec Vérification de Message & Typage
Certains tests valident précisément le type d'erreur et le message :
- [`test_section_b_typed_errors_on_arithmetic_overflow`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/lib.rs#L44) :
  ```rust
  assert_eq!(res.unwrap_err().kind, DFDLErrorKind::TypeError);
  ```
- [`test_section_c_xml_undeclared_prefix_error`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/lib.rs#L168) :
  ```rust
  assert_eq!(res.unwrap_err().kind, DFDLErrorKind::Parse);
  ```

### 4.2. Tests Faibles avec Assertion "Nue" (`assert!(res.is_err())`)
Plusieurs tests majeurs vérifient uniquement que le résultat est une erreur, sans tester ni la variante de `DFDLErrorKind` ni le texte du message :
- [`test_section_j_sequence_missing_separator_error`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/lib.rs#L862-L895) (Ligne 894) :
  ```rust
  let res = parser.parse_document();
  assert!(res.is_err()); // Ne vérifie pas si l'erreur est un séparateur manquant ou un EOF !
  ```
- [`test_section_k_array_min_occurs_violation_error`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/lib.rs#L1028-L1060) (Ligne 1059) :
  ```rust
  let res = parser.parse_document();
  assert!(res.is_err()); // Ne vérifie pas la violation de minOccurs !
  ```
- [`test_section_l_choice_all_branches_failed_error`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/lib.rs#L1183-L1216) (Ligne 1215) :
  ```rust
  let res = parser.parse_document();
  assert!(res.is_err()); // Ne vérifie pas l'échec de toutes les branches du choix !
  ```

---

## 5. Normalisation et Formatage des Messages d'Erreur

### 5.1. Formatage `Display` vs Recherche de Sous-chaînes
- Dans [`crates/dfdl-core/src/error.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/error.rs#L132-L143) :
  ```rust
  impl fmt::Display for DFDLError {
      fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
          match &self.location {
              Some(loc) => write!(f, "[{:?}] at offset {}: {}", self.kind, loc.byte_offset, self.message),
              None => write!(f, "[{:?}]: {}", self.kind, self.message),
          }
      }
  }
  ```
- `[{:?}]` formate `[SchemaDefinition]` ou `[Parse]`.
- Les suites de tests Apache Daffodil recherchent souvent la chaîne littérale `"Schema Definition Error"` ou `"Parse Error"`.
- **Bonne pratique observée :** Dans `dfdl-schema`, les développeurs ont explicitement préfixé les messages eux-mêmes par `"Schema Definition Error: ..."` (plus de 200 occurrences dans le code), et dans `dfdl-core` par `"Parse Error: ..."` (plus de 80 occurrences). Cela permet aux sous-chaînes de matcher même si l'en-tête de `Display` utilise `[{:?}]`.

---

## 6. Synthèse des Écarts et Risques de Faux Positifs

| Emplacement | Problème Détecté | Risque / Sévérité | Impact Concret sur la Conformité |
|:---|:---|:---:|:---|
| `tdml.rs:1409` | `matches_expected \|\| !tc.expected_errors.is_empty()` | **Critique** | Tous les tests parser attendant une erreur passent, même avec un message erroné. |
| `tdml.rs:1222` | Échec de schéma passe sans vérifier `tc.expected_errors`. | **Critique** | Masque des schémas invalides ou confond SDE et Processing Error. |
| `tdml.rs:1542` | `unparse_res.is_err()` passe sans vérification de message. | **Majeur** | Masque les causes réelles d'échec à l'unparsing. |
| `dfdl-tests/src/lib.rs` | Assertions nues `assert!(res.is_err())` sur les séparateurs et `minOccurs`. | **Moyen** | Fragilité des tests de non-régression (succès possible sur un mauvais échec). |

---

## 7. Plan d'Action Recommandé pour l'Exactitude des Tests

```
ÉTAPE 1 : Rétablissement de la Vérification Stricte dans le Runner TDML
├── Dans `tdml.rs:1416` : Remplacer `if matches_expected || !tc.expected_errors.is_empty()`
│   par `if matches_expected` (échec strict si la sous-chaîne attendue n'est pas trouvée).
├── Dans `tdml.rs:1222` : Vérifier que le message d'erreur de compilation du schéma
│   contient l'une des sous-chaînes de `tc.expected_errors`.
└── Dans `tdml.rs:1542` : Vérifier que l'erreur d'unparse correspond aux sous-chaînes attendues.

ÉTAPE 2 : Durcissement des Tests Directs (`crates/dfdl-tests/src/lib.rs`)
├── Remplacer `assert!(res.is_err())` par :
│   1. `assert_eq!(err.kind, DFDLErrorKind::Parse)`
│   2. `assert!(err.message.as_str().contains("..."))`
└── Vérifier systématiquement la conformité du code d'erreur sur :
    - `test_section_j_sequence_missing_separator_error` ("separator")
    - `test_section_k_array_min_occurs_violation_error` ("minOccurs")
    - `test_section_l_choice_all_branches_failed_error` ("All choice alternatives failed")

ÉTAPE 3 : Distinction Stricte entre SDE et PE
└── Dans TDML, vérifier que les tests contenant `<tdml:error>Schema Definition Error</tdml:error>`
    échouent bien avec `DFDLErrorKind::SchemaDefinition` et NON `DFDLErrorKind::Parse`.
```

---

## 8. Conclusion de l'Audit

L'audit révèle un contraste saisissant :
1. **Les tests unitaires du compilateur (`dfdl-schema`) sont d'une exactitude irréprochable**, vérifiant systématiquement le contenu précis des diagnostics d'erreurs et les références au standard.
2. **Le harnais de test TDML (`dfdl-tests/src/tdml.rs`) comporte un assouplissement ("Soft-Pass")** qui accepte n'importe quelle erreur dès lors qu'une erreur est attendue.

Corriger la condition de ligne 1416 et ligne 1222 dans `tdml.rs` permettra de mesurer le **taux d'exactitude réel** des messages d'erreur du moteur par rapport aux attentes normatives d'Apache Daffodil, garantissant qu'aucune régression silencieuse ne puisse subsister.
