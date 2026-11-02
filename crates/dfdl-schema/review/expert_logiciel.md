# Revue d'Expertise Génie Logiciel & Architecture : `dfdl-schema`

- **Date :** 2026-10-28
- **Horodatage :** 17:47:26 UTC+1
- **Cible :** `crates/dfdl-schema` (Compilateur Frontend DFDL)
- **Auteur :** Expert Architecture Logicielle & Qualité de Code

---

## 1. Architecture Globale du Compilateur & Pipeline

Le crate `dfdl-schema` implémente un compilateur classique à passes successives, séparant nettement l'analyse syntaxique XSD de l'arbre IR exécutable final :

```
    [Schémas XSD + DFDL]
             │
             ▼  Passe 1 : Parsing Streaming XML (dfdl-xml)
     [XsdSchemaDoc]
             │
             ▼  Passe 2 : Résolution d'Inclusions & Imports (Chameleon / TargetNS)
       [XsdSchema]
             │
             ▼  Passe 3 : Passes de Validation Sémantique (Facettes, UPA, Assertions, Polymorphisme)
  [Valid XsdSchema AST]
             │
             ▼  Passe 4 : Abaissement & Résolution des Propriétés DFDL (lower.rs)
   [CompiledSchema IR] (dfdl-core)
```

### Points forts de cette architecture :
1. **Découplage Temporel Fort :** La compilation s'opère "Ahead-of-Time" (ou une seule fois au démarrage). L'objet [`CompiledSchema`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs) produit est complètement immuable, thread-safe (`Send + Sync`), et totalement débarrassé des concepts XML/XSD (balises, namespaces, commentaires).
2. **Découplage de Dépendances :** `dfdl-core` n'a aucune dépendance vers `dfdl-schema`. Les moteurs d'exécution (`ParserEngine`, `UnparserEngine`, `dfdl-vm`) ne dépendent que de `CompiledSchema`.

---

## 2. Analyse de la Dette Technique & Complexité du Code

### 2.1. Le Phénomène "God Module" dans `lower.rs`
- **Constat chiffré :**
  - [`src/compiler/lower.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler/lower.rs) regroupe **3 600 lignes de code**.
  - Il cumule :
    - La résolution des types d'éléments simples et complexes.
    - La conversion des types XSD primitifs vers les types scalaires DFDL.
    - L'injection et le calcul des délimiteurs, préfixes, et politiques de cadrage.
    - L'évaluation des expressions de longueurs et d'occurrences.
    - La construction du graphe de nœuds [`NodeId`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs).
- **Conséquences :**
  - Complexité cyclomatique locale élevée.
  - Risque d'effets de bord lors de modifications de règles transversales.
  - Augmentation du temps de compilation LLVM.
- **Plan de Refactorisation Recommandé :**
  Scinder `lower.rs` en un dossier `compiler/lower/` comportant :
  - `lower/element.rs` : Abaissement des éléments et types dérivés.
  - `lower/groups.rs` : Abaissement des séquences et choix.
  - `lower/properties.rs` : Résolution et normalisation des propriétés DFDL.
  - `lower/validation.rs` : Vérifications de cohérence IR post-abaissement.

### 2.2. Abstraction du Résolveur de Schémas Externe
- **État actuel :**
  - Dans [`compiler.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler.rs#L225), la résolution des fichiers inclus/importés s'effectue via une fermeture :
    ```rust
    pub fn compile_str_with_resolver<R>(&self, xml: &str, resolver: R) -> DFDLResult<CompiledSchema>
    where
        R: Fn(&str) -> Option<String>,
    ```
- **Limites architecturales :**
  - Impossibilité de gérer facilement un cache de schémas compilés ou d'ASTs intermédiaires.
  - Absence de support natif pour les catalogues XML (OASIS XML Catalogs) couramment utilisés pour réécrire les URIs canoniques en chemins relatifs ou ressources embarquées.
- **Recommandation :**
  Définir un trait formel :
  ```rust
  pub trait SchemaResolver {
      fn resolve_schema(&self, location: &str, base_uri: Option<&str>) -> DFDLResult<Option<String>>;
  }
  ```

---

## 3. Stratégie de Test et Assurance Qualité

- **Bilan des tests existants :**
  - Le fichier [`compiler/tests.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler/tests.rs) contient **114 tests unitaires complets** exécutés en 0.02s.
  - Excellente proportion de tests négatifs vérifiant le rejet immédiat des schémas non conformes (`SchemaDefinitionError`).
- **Pistes d'amélioration :**
  - Découpler les tests unitaires volumineux de `compiler/tests.rs` (156 Ko) en sous-fichiers de tests d'intégration thématiques dans un répertoire `tests/`.
  - Automatiser la couverture de schémas réels (ASTERIX, FakeTDL, PCAP, formats aéronautiques).

---

## 4. Tableau Synthétique des Recommandations Logicielles

| Domaine | Problématique | Criticité | Action Recommandée |
|:---|:---|:---:|:---|
| **Structure** | `lower.rs` (3600 lignes) concentre trop de logiques disparates. | Élevée | Scinder en sous-modules thématiques dans `compiler/lower/`. |
| **Extensibilité** | Résolveur de schémas basé sur une fermeture `Fn(&str)`. | Moyenne | Introduire un trait `SchemaResolver` avec support de cache. |
| **Organisation** | `compiler/tests.rs` monolithique (156 Ko). | Faible | Déplacer vers des tests d'intégration dans `tests/`. |

---

## 5. Conclusion Générale Logiciel
`dfdl-schema` présente une architecture de compilation remarquable, avec un découpage frontend/IR très net garantissant que le moteur d'exécution opère sans aucune surcharge XML. Le principal chantier d'ingénierie logicielle consiste à réorganiser le module `lower.rs` pour pérenniser sa maintenabilité à long terme.
