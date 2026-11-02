# Revue Technique : Suppression du codage en dur de `max_skip_limit`

**Date & Heure :** 2026-10-28 20:52:00  
**Auteur :** Antigravity Pair Programmer  
**Composants impactés :** [`dfdl-core`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core), [`dfdl-schema`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema), [`dfdl-tests`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-tests)

---

## 1. Contexte & Problématique

Lors de la résolution du cas de test TDML `leadingSkip2` (qui teste la détection d'un dépassement de la limite maximale autorisée pour un saut d'alignement ou de positionnement), une variable locale en dur avait été initialement introduite dans [`crates/dfdl-core/src/expr/properties.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/properties.rs) :
```rust
let max_skip_limit: usize = 1024;
```

Comme souligné à juste titre, **coder des limites de ressources en dur directement dans le code métier viole les principes d'architecture de DFDL et les règles d'ingénierie logicielle** :
- Cette valeur correspond au paramètre ajustable (tunable) officiel Daffodil / DFDL `maxSkipLengthInBytes`.
- Dans des environnements contraints (embarqué `#![no_std]`, streaming haute performance, sécurité contre les dénis de service / DoS), les utilisateurs doivent pouvoir configurer ou contraindre explicitement ces seuils.
- L'infrastructure existante de `dfdl-core` possède déjà une structure dédiée aux plafonds de mémoire et d'allocation : [`ResourceLimits`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/limits.rs#L7).

---

## 2. Refactorisation Réalisée

### 2.1. Promotion au niveau de [`ResourceLimits`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/limits.rs#L7)
- Définition d'une constante explicite et documentée :
  ```rust
  pub const DEFAULT_MAX_SKIP_LENGTH_IN_BYTES: usize = 1024;
  ```
- Ajout du champ `pub max_skip_length_in_bytes: usize` dans [`ResourceLimits`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/limits.rs#L7) avec initialisation par défaut à `DEFAULT_MAX_SKIP_LENGTH_IN_BYTES`.
- Ajout de la méthode de validation dédiée :
  ```rust
  #[inline]
  pub fn check_skip_length(&self, skip_bytes: usize) -> bool {
      skip_bytes <= self.max_skip_length_in_bytes
  }
  ```

### 2.2. Intégration dans la Représentation Intermédiaire ([`CompiledSchema`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs#L1730) & [`SchemaBuilder`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/builder.rs#L12))
- [`SchemaBuilder`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/builder.rs#L12) et [`CompiledSchema`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs#L1730) transportent désormais la configuration active du `max_skip_length_in_bytes`.

### 2.3. Résolution des Propriétés avec Limites ([`PropertyStore`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/properties.rs))
- Remplacement du calcul local par :
  ```rust
  pub fn to_resolved_properties_with_limits(
      &self,
      parent: Option<&PropertyStore>,
      limits: &crate::limits::ResourceLimits,
  ) -> DFDLResult<ResolvedProperties>
  ```
- Validation via `limits.check_skip_length(skip_bytes)` :
  ```rust
  if !limits.check_skip_length(skip_bytes) {
      let msg = alloc::format!(
          "Schema Definition Error: Skip length {} exceeds tunable maxSkipLengthInBytes {}",
          skip_bytes,
          limits.max_skip_length_in_bytes
      );
      return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
  }
  ```
- La méthode `to_resolved_properties` historique reste disponible pour compatibilité et délègue à `to_resolved_properties_with_limits(parent, &ResourceLimits::default())`.

### 2.4. Exposition dans [`SchemaCompiler`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler.rs#L40)
- Ajout du champ `pub max_skip_length_in_bytes: usize` dans [`SchemaCompiler`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler.rs#L40).
- Méthode fluent de configuration :
  ```rust
  pub fn with_max_skip_length_in_bytes(mut self, limit: usize) -> Self
  ```
- Méthode d'accès aux limites :
  ```rust
  pub fn resource_limits(&self) -> dfdl_core::limits::ResourceLimits
  ```
- Propagation systématique à l'ensemble du pipeline de compilation et de lowering (`lower_element`, `lower_sequence`, `lower_choice`, etc.).

---

## 3. Résultats de Validation

| Contrôle | Commande | Résultat |
| :--- | :--- | :--- |
| **Compilation Workspace** | `cargo check --workspace --all-targets` | **0 erreur** (11/11 crates) |
| **Analyse Statique Clippy** | `cargo clippy --workspace --all-targets -- -D warnings` | **0 avertissement** |
| **Tests Unitaires Cœur** | `cargo test -p dfdl-core -p dfdl-schema` | **413 passés, 0 échec** |
| **Suite Officielle Daffodil TDML** | `cargo test -p dfdl-tests -- test_apache_daffodil_official_tdml_suite` | **100.0% (4 336 / 4 336 passés)** |
| **Graphe Contextuel Graft** | `graft build` | **À jour (1 961 nœuds, 3 881 arêtes)** |
