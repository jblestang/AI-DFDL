# Analyse de Performance Roundtrip Unparse/Parse FakeTDL
**Horodatage :** 2026-10-29T21:30:00+01:00  
**Binary :** `dfdl-benchmark` (Release build, Apple Silicon M-series)  
**Méthodologie :** Profilage par échantillonnage macOS `sample` à 1 kHz sur 300 000 itérations du benchmark `roundtrip_faketdl`.

---

## 1. Répartition Globale du Temps (Roundtrip)

Sur l'ensemble du roundtrip unparse/parse (`roundtrip_faketdl`) :
- **Total des échantillons :** ~2 681 samples
- **Phase Unparse (`unparse_faketdl`) :** **~60.4%** (1 619 samples)
- **Phase Parse (`parse_faketdl`) :** **~39.6%** (1 062 samples)

Le coût du roundtrip est donc dominé par l'**Unparser** (~1.5× plus coûteux que le parser).

---

## 2. Profil Détaillé de l'Unparser (1 619 samples)

### Goulot d'étranglement #1 : Allocation systématique dans `validate_unparse_value` (~187 samples, ~11.5% de l'unparse)
- **Fichier & Ligne :** `crates/dfdl-core/src/kernel/unparser/element.rs:97-98`
```rust
let str_rep = alloc::format!("{}", val);
let trimmed = str_rep.trim();
```
- **Diagnostic :**
  Pour **chaque** élément simple non-complexe du message (coordonnées x, y, z, id, flag, timestamp, etc.), l'unparser alloue une `String` sur le tas via `alloc::format!("{}", val)` pour vérifier les bornes ou les signes.
  - Pour les flottants (`f32`, `f64`), cela appelle l'algorithme Grisu de conversion flottant-vers-décimal (`flt2dec::strategy::grisu::format_shortest_opt`, 43 samples).
  - Cela déclenche systématiquement un appel à `malloc` (`malloc_zone_malloc`) suivi de `free` (`_xzm_free`).
  - **Absurdité :** Dans 99.9% des cas nominaux valides, cette chaîne n'est jamais utilisée en dehors des messages d'erreur ou d'un fallback si `as_i128()` échoue ! Les tests de signe (`val.is_negative()`) et de bornes peuvent être faits directement en arithmétique native sans aucune allocation de chaîne.

### Goulot d'étranglement #2 : Formatage du calendrier binaire (`unparse_binary_calendar`, ~58 samples)
- **Fichier :** `crates/dfdl-core/src/kernel/parser/calendar.rs` (`format_calendar_with_pattern`, `parse_tz_offset_minutes`)
- **Diagnostic :**
  - Instanciation répétée de `StrSearcher` et allocations intermédiaires (`Vec<char>`, `Vec<&str>`) pour analyser le motif de date/heure et le fuseau horaire.
  - Multiples formatages de sous-composants numériques via `write!`.

### Goulot d'étranglement #3 : Gestion des segments et suspensions de l'unparser
- **Diagnostic :**
  - Les closures de suspension et la fusion de tampons (`unparse_element_with_seg_internal`).

---

## 3. Profil Détaillé du Parser (1 062 samples)

### Goulot d'étranglement #1 : Construction de l'arbre Infoset DOM (~120 samples)
- **Fichiers :** `crates/dfdl-core/src/infoset/tree.rs` (`push_event_with_hidden`, `try_add_child`)
- **Diagnostic :**
  - Chaque élément simple ou complexe parsed crée un `InfosetElement`.
  - Duplication de chaînes de caractères pour les noms d'éléments et namespaces (`String::clone`, ~48 samples).
  - Réallocations de vecteurs d'enfants (`Vec::push` déclenchant `finish_grow` / `realloc`, ~33 samples).

### Goulot d'étranglement #2 : Décodage et parsing de texte (`parse_text_value`, ~70 samples)
- **Fichiers :** `crates/dfdl-core/src/kernel/parser/text.rs` & `crates/dfdl-core/src/encoding.rs`
- **Diagnostic :**
  - `decode_text_bytes` alloue une nouvelle `String` pour les champs texte ou délimités.
  - Calcul répété de `encoding_mandatory_alignment_bits` avec découpage et trim de chaînes.

### Goulot d'étranglement #3 : Lecture binaire et calendriers (`parse_binary_value`, ~60 samples)
- **Diagnostic :**
  - Lecture bit à bit (`BitReader::read_bits`) et reconstruction des entiers/flottants.
  - `parse_calendar_from_digits` utilise `alloc::format!` pour formater la chaîne ISO 8601 intermédiaire.

---

## 4. Recommandations et Gains Potentiels

1. **Suppression de l'allocation eagerly dans `validate_unparse_value` (Gain estimé : ~8-10% sur le roundtrip) :**
   - Remplacer le `alloc::format!("{}", val)` systématique par une validation purement numérique directe sur `DfdlValue` (`as_i128()`, `as_f64()`, `is_negative()`).
   - N'allouer une représentation textuelle que dans les branches d'erreur (chemins `Err(...)`).

2. **Élimination des clones de noms dans l'Infoset DOM (Gain estimé : ~3-5% sur le parse) :**
   - Utiliser des `Arc<str>` ou des références partagées/internées pour les noms de balises du schéma plutôt que de cloner des `String` heap à chaque occurrence d'élément.

3. **Optimisation du calendrier binaire / fuseaux horaires (Gain estimé : ~2-3%) :**
   - Remplacer les décompositions de chaînes dynamiques pour le fuseau par une extraction statique sans allocations.
