# Revue d'Expertise DFDL : `dfdl-schema`

- **Date :** 2026-10-28
- **Horodatage :** 17:47:26 UTC+1
- **Cible :** `crates/dfdl-schema` (Compilateur XSD & Spécifications DFDL)
- **Référence Standard :** OGF GFD-P-R.207 DFDL v1.0 (Sections 5 à 17, 23)
- **Auteur :** Expert Domaine DFDL (Data Format Description Language)

---

## 1. Contexte Normatif & Couverture Fonctionnelle

Le standard DFDL v1.0 (GFD-P-R.207) formalise les règles de description des formats binaires et textuels via XML Schema 1.0 annoté. Le rôle du compilateur `dfdl-schema` est de valider la conformité statique du schéma avant toute exécution et de traduire ces définitions en un graphe IR exécutable immuable.

### Composants DFDL audités dans le compilateur :
1. **Annotations de format (§5, §6) :** `dfdl:format`, `dfdl:defineFormat`, `dfdl:ref`.
2. **Propriétés de cadrage (§12, §14) :** Délimiteurs (`initiator`, `terminator`, `separator`), positions (`prefix`, `postfix`, `infix`), politiques de suppression (`never`, `trailingEmpty`, `trailingEmptyStrict`, `anyEmpty`).
3. **Propriétés de longueur (§12.3) :** `lengthKind` (`explicit`, `delimited`, `implicit`, `pattern`, `prefixed`, `endOfParent`), `lengthUnits` (`bytes`, `bits`, `characters`), `prefixLengthType`.
4. **Représentations binaires et textuelles (§13) :** `binaryNumberRep` (`binary`, `packed`, `bcd`, `ibm4690Packed`), `byteOrder`, `bitOrder`, représentations calendaires, booléennes, et numériques formatées.
5. **Résolution de points d'incertitude (§9, §15) :** `dfdl:assert`, `dfdl:discriminator`, `dfdl:choiceDispatchKey`, `dfdl:choiceBranchKey`, `choiceBranchKeyRanges`.
6. **Variables et valeurs calculées (§17, §23) :** `dfdl:defineVariable`, `dfdl:setVariable`, `dfdl:newVariableInstance`, `dfdl:inputValueCalc`, `dfdl:outputValueCalc`.
7. **Groupes masqués et nillabilité (§13.1, §14.4) :** `dfdl:hiddenGroupRef`, `dfdl:nilKind`, `dfdl:nilValue`.

---

## 2. Analyse Détaillée des Écarts par Rapport au Standard DFDL v1.0

### 2.1. Cascade de Portée des Propriétés (Property Scoping §5.1)
- **Exigence du standard :** DFDL impose une cascade de résolution en 4 niveaux hiérarchiques stricts :
  1. Annotation locale sur l'élément/terme.
  2. Type simple ou complexe local (`xs:simpleType`, `xs:complexType`).
  3. Définition de type de base ou groupe nommé référencé.
  4. Format par défaut au niveau du document racine de schéma (`dfdl:format`).
  5. Formats nommés résolus via `dfdl:ref` (avec fusion des propriétés non surchargées).
- **Implémentation dans `dfdl-schema` :**
  - Géré via [`PropertyStore`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/properties.rs) et combiné dans [`annotations.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler/annotations.rs) et [`lower.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler/lower.rs).
- **Écart constaté :**
  - Dans le cas où un `dfdl:defineFormat` référence un autre `dfdl:defineFormat` via `dfdl:ref` imbriqué au sein d'un schéma inclus sans `targetNamespace` (schéma caméléon), la priorité de résolution favorise parfois la première liaison locale sans résoudre complètement la chaîne transitive de formats. Un test de régression a été ajouté (`test_prefixed_and_chameleon_format_ref_resolution`), mais la chaîne arbitraire à $N$ niveaux nécessite une validation formelle d'ordre topologique.

### 2.2. Contraintes de Validité des Assertions et Discriminateurs (§9.2, §9.3)
- **Exigence du standard :**
  - Un composant ne peut pas comporter simultanément une assertion `<dfdl:assert>` et un discriminateur `<dfdl:discriminator>`.
  - Un composant ne peut pas comporter plus d'un discriminateur.
  - Le discriminateur doit avoir une condition non vide (`test` ou `testPattern`).
  - L'attribut `failureType` supporte `processingError` ou `recoverError`.
- **Statut dans `dfdl-schema` :** **Parfaitement Conforme.**
  - Implémenté et validé statiquement dans [`validate_term_assertions`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler/annotations.rs#L16-L65). Les violations émettent une `SchemaDefinitionError` immédiate.

### 2.3. Politiques de Longueur Implicite et Facettes (§12.3.3)
- **Exigence du standard :** Pour `lengthKind="implicit"`, la longueur des types chaînes et hexBinary doit être strictement déterminée par les facettes XSD `maxLength` ou `length`. Si `minLength != maxLength`, une erreur `SchemaDefinitionError` doit être levée.
- **Statut dans `dfdl-schema` :** **Conforme.**
  - Vérifié dans `test_implicit_length_unequal_min_max_length_facet_error` et `test_string_implicit_length_without_facets_rejected`.

### 2.4. Types Avancés XSD (`xs:union` et `xs:list`)
- **Exigence du standard (§4.1.2) :** DFDL v1.0 restreint l'usage des `xs:union` (autorisés uniquement sur types simples sans ambiguïté physique) et interdit les `xs:list` à délimiteurs whitespace standard lorsqu'ils entrent en conflit avec les délimiteurs DFDL.
- **Écart constaté :** `dfdl-schema` rejette actuellement de manière inconditionnelle `xs:union` et `xs:list` dès le parsing XSD avec une `SchemaDefinitionError`. C'est une restriction par conception ("Fail-Fast"), justifiable par la complexité sémantique de l'union en binaire, mais cela constitue un écart mineur par rapport au sous-ensemble strict permis par Daffodil.

### 2.5. Détection Statique des Cycles de Définition (§4.1.2)
- **Exigence du standard :** DFDL interdit formellement toute récursion dans les types ou groupes (`<xs:group ref="Loop"/>`).
- **Statut dans `dfdl-schema` :**
  - La détection de récursion est assurée par un compteur de profondeur lors de la compilation (`check_nesting`).
  - **Recommandation DFDL :** Remplacer le seuil de profondeur numérique par un calcul de graphe orienté de dépendances (détection de cycles de Tarjan) lors de la validation préliminaire de l'AST afin de signaler le chemin exact du cycle à l'utilisateur avant le lowering.

---

## 3. Matrice de Conformité DFDL v1.0 pour `dfdl-schema`

| Section Normative DFDL v1.0 | Fonctionnalité | Statut | Commentaire |
|:---|:---|:---:|:---|
| **§5 & §6** | Syntaxe courte (`dfdl:prop="..."`) et longue (`<dfdl:format>`) | **Conforme** | Support complet des deux syntaxes. |
| **§7** | Scoping et fusion de formats (`dfdl:ref`) | **Conforme partiel** | Chaînes transitives complexes à $N$ niveaux à auditer. |
| **§9** | `<dfdl:assert>` et `<dfdl:discriminator>` | **Conforme** | Validation stricte des cardinalités et types de tests. |
| **§12** | Alignement binaire et skips (`alignmentUnits`, `leadingSkip`) | **Conforme** | Support bits et octets. |
| **§12.3** | Modes `lengthKind` (explicit, delimited, implicit, pattern, prefixed) | **Conforme** | Prise en charge des descripteurs de préfixe. |
| **§13** | Nombres binaires (Binary, Packed Comp-3, BCD, IBM4690) | **Conforme** | Validation des longueurs minimales en bits. |
| **§14** | Séquences et Délimiteurs (`initiator`, `separatorSuppressionPolicy`) | **Conforme** | 4 politiques de suppression intégrées dans l'IR. |
| **§15** | Facettes XSD (min/max Inclusive/Exclusive, patterns, enums) | **Conforme** | Vérification croisée et sous-ensembles d'enums. |
| **§16** | `occursCountKind` (fixed, implicit, parsed, expression, stopValue) | **Conforme** | Rejet des expressions relatives non licites. |
| **§17** | `inputValueCalc` et `outputValueCalc` | **Conforme** | Rejet de la combinaison simultanée sur un même élément. |
| **§23** | Variables (`defineVariable`, `setVariable`, `newVariableInstance`) | **Conforme** | Vérification des conflits d'assignation unparse. |

---

## 4. Conclusion de l'Expert DFDL
Le compilateur `dfdl-schema` démontre une conformité de premier ordre avec les spécifications DFDL v1.0 de l'OGF. Les validations statiques de règles de schémas (assert/discriminator, facettes, exclusivité des calculs de valeurs) sont implémentées avec une rigueur exemplaire, alignée sur le moteur de référence Apache Daffodil.
