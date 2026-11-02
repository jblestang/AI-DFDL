# Rapport d'Expertise et Revue de Conformité : Suites DFDL NTP et MIL-STD-2045

**Date :** 2026-10-29 07:48:00 UTC  
**Auteur :** Antigravity (AI-DFDL Engineering Team)  
**Domaines d'expertise :** Protocoles Réseaux (NTP, RFC-5905), Liaisons de Données Tactiques & Systèmes Militaires (MIL-STD-2045-47001D/D1), Spécification OGF DFDL v1.0, Ingénierie Logicielle Rust & Sécurité mémoire (`no_std`).  
**Périmètre :** Résolution complète des tests échoués sur les suites externes officielles DFDLSchemas `ntp` et `mil-std-2045`.

---

## 1. Synthèse Exécutive des Résultats

| Suite Externe | Dépôt source | Tests Totaux | Tests Réussis Avant | Tests Réussis Après | Taux de Succès Final |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **NTP** (Network Time Protocol) | `DFDLSchemas/ntp` | 5 | 2 | **5** | **100.0%** (5/5) |
| **MIL-STD-2045** (Application Header) | `DFDLSchemas/mil-std-2045` | 33 | 18 | **33** | **100.0%** (33/33) |
| **Total Combiné** | | **38** | **20** | **38** | **100.0%** (38/38) |

- **Suite interne de régression `dfdl-core` :** 297/297 passés (100%).
- **Suite interne `dfdl-schema` :** 117/117 passés (100%).
- **Contrôle Qualité Clippy :** `cargo clippy --workspace --all-targets -- -D warnings` -> **0 avertissement, 0 erreur**.
- **Mode `no_std` :** Respect strict de `#![no_std]` dans [`crates/dfdl-core`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core).

---

## 2. Analyse Approfondie des Défaillances et Solutions Techniques

### A. Suite NTP (`TestNtp.tdml` - 100% de succès)

#### 1. Résolution des artefacts compilés Scala/SBT (`*.bin`)
- **Problème :** Trois tests (`test_ntp_v4_raw`, `test_ntp_v3`, `test_ntp_v4`) spécifiaient `model="dfdl-ntp.bin"` ou `model="ntp.bin"`. Dans l'écosystème Daffodil SBT, ce modèle est un artefact binaire sérialisé.
- **Expertise :** Analyse de `build.sbt` qui associe chaque `*.bin` à sa définition source XSD (`DaffodilBinInfo(...)` ou `schema = "..."`).
- **Correctif dans [`crates/dfdl-tests/src/tdml.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests/src/tdml.rs) :**
  Extraction automatique de la correspondance schéma depuis `build.sbt` dans le répertoire du schéma pour charger le fichier `.dfdl.xsd` canonique correspondant (`com/owlcyberdefense/ntp/xsd/ntp.dfdl.xsd`).

#### 2. Support des entrées document et infoset `type="file"`
- **Problème :** Les tests NTP référencent des fichiers de test externes via `<documentPart type="file">` et `<dfdlInfoset type="file">`.
- **Correctif :** Résolution robuste des chemins relatifs au fichier TDML et au `base_dir`, chargement des octets bruts et parsing de l'arbre XML attendu.

#### 3. Héritage des propriétés physiques via `dfdlx:repType`
- **Problème :** L'élément NTP `stratum` est typé avec un type d'énumération lié à un type physique sous-jacent via `dfdlx:repType="tns:unsignedByte"`. Le compilateur n'héritait pas des propriétés physiques de formatage binaire de ce type sous-jacent.
- **Correctif :** Propagation des propriétés d'alignement, d'encodage et de longueur issues du `repType`.

---

### B. Suite MIL-STD-2045 (`milstd2045.tdml` - 100% de succès)

La spécification MIL-STD-2045-47001 (Application Header) est l'un des formats binaires militaires les plus contraignants :
- Encodage sous-octet `X-DFDL-US-ASCII-7-BIT-PACKED` (7 bits par caractère sans padding par caractère).
- Ordre des bits `LSBFirst` combiné avec des entités de contrôle ASCII (`%DEL;` = `0x7F`, 7 bits `1111111`).
- Présence d'indicateurs de présence de 1 bit (`ms2045:presenceIndicator`), de groupes conditionnels `dfdl:occursCount="{ if (...) then 1 else 0 }"`, et de groupes `futureUseGroup1..3` composés uniquement de champs cachés (`dfdl:hiddenGroupRef`) calculés via `dfdl:outputValueCalc`.

#### 1. Décodage et alignement des Délimiteurs en Encodage Sub-Byte (7-bit packing)
- **Fichier :** [`crates/dfdl-core/src/kernel/unparser/framing.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/framing.rs)
- **Constat & Problème :** Dans `tString64`, la chaîne texte a pour terminateur une expression DFDL `{if (fn:string-length(./value) eq 64) then '%ES;' else '%DEL;'}`.
  1. Lors de l'unparse, `%DEL;` n'était pas décodé en caractère de contrôle `\x7F`, écrivant les caractères littéraux `"%DEL;"` (5 octets au lieu de 7 bits).
  2. Même décodé en `\x7F`, `write_evaluated_delimiter` forçait l'écriture sur 8 bits (`write_bits(b, 8)`), ce qui créait un décalage de bit irréversible pour tout le reste du message dans un flux `X-DFDL-US-ASCII-7-BIT-PACKED`.
- **Solution :** Utilisation de `crate::encoding::encoding_char_bits(&props.encoding)` pour déterminer la taille unitaire exacte en bits du caractère (7 bits pour `X-DFDL-US-ASCII-7-BIT-PACKED`), et délégation des entités DFDL à `decode_dfdl_character_entities`.
- **Impact :** Les 13 premiers octets (104 bits) de l'en-tête sont devenus 100% identiques bit-à-bit à l'en-tête attendu.

#### 2. Déréférencement des Groupes Masqués et Suppression de Séparateurs
- **Fichier :** [`crates/dfdl-core/src/kernel/unparser/mod.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/mod.rs)
- **Constat & Problème :** Aux bits 104..192 (octets 13..23), les groupes de futurs usages (`futureUseGroup1`, `futureUseGroup2`, `futureUseGroup3`) étaient entièrement ignorés par l'unparser.
  - L'élément `<future1/>` présent dans l'infoset XML a `state == ElementState::NoValue` et ne contient aucun enfant XML direct (ses enfants sont tous définis dans le schéma via `dfdl:hiddenGroupRef="ms2045:future_use_n_PI"` avec `dfdl:outputValueCalc="{0}"`).
  - La fonction `is_empty_infoset_element` considérait à tort qu'un élément sans enfant XML était "vide".
  - La condition de boucle de séquence exécutait `if suppress_sep { continue; }`, éliminant l'unparsing de `<future1/>`, `<future2/>` et `<future3/>`. De plus, cette suppression était appliquée même sur des séquences sans séparateur (`sep_opt.is_none()`).
- **Solution :**
  1. Un élément complexe (`CompiledType::Complex`) présent dans l'infoset XML n'est jamais considéré comme vide lors du test `is_empty_infoset_element`, car ses structures de schéma (groupes masqués, calculs OVC) génèrent des bits physiques sur le support.
  2. La suppression de séparateur ne peut avoir lieu que si un séparateur est effectivement configuré (`sep_opt.is_some()`).
  3. Rétablissement des 15 bits masqués (5 bits par groupe de futurs usages).
- **Résultat :**
  - `test2045msghdr1_unparsed` : **SUCCÈS (24/24 octets bit-à-bit identiques)**.
  - `test2045D1_operator_ind_on_unparsed` : **SUCCÈS (24/24 octets)**.
  - `test_2045_D1_all_fields_x2u` : **SUCCÈS (566/566 octets)**.

#### 3. Évaluation XPath et Chemins Relatifs Ascendants
- **Fichier :** [`crates/dfdl-core/src/expr/eval/navigation.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval/navigation.rs)
- **Solution :** Prise en charge dans l'indexation de navigation relative XPath de `OccursCountKind::Expression` et `StopValue` dans `is_array` pour évaluer fidèlement les prédicats tels que `{ ../recipient_address_group_PI/value }` et `{ ../header_size_PI/value }`.

---

## 3. Matrice de Conformité et Bonnes Pratiques

- **Karpathy Rule 1 (Think Before Coding) :** Décomposition rigoureuse du flux binaire au niveau du bit (analyse LSBFirst bit 104 vs bit 192).
- **Karpathy Rule 2 (Simplicity First) :** Utilisation des fonctions d'encodage existantes de `dfdl-core` sans dépendance externe superflue.
- **Karpathy Rule 3 (Surgical Changes) :** Correction ciblée dans `framing.rs` et `unparser/mod.rs` sans modifier le comportement des schémas textuels ou délimités standards.
- **Karpathy Rule 4 (Goal-Driven Execution) :** 100% de tests verts sur `ntp` (5/5) et `mil-std-2045` (33/33).
- **Karpathy Rule 5 (Domain Expertise) :** Expertise poussée sur le format militaire MIL-STD-2045 (empaquetage 7-bit, ordres de bits LSBFirst, indicateurs de présence).
- **Karpathy Rule 6 (Review & Tracking) :** Production du présent rapport de revue horodaté.
