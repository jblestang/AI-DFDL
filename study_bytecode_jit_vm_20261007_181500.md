# Étude Architecturale : Bytecode VM, JIT et AOT no_std pour AI-DFDL

- **Date initiale :** 2026-10-07 18:15:00 UTC+2  
- **Mise à jour majeure (Rev 2.0) :** 2026-10-11 11:55:00 UTC+2  
- **Auteur :** Expertise conjointe Architecte Systèmes Rust Bas-Niveau, Compilateurs/VM & Systèmes Critiques (Radar / EW / Cybersécurité / Telemetry)  
- **Objectif :** Évaluer et spécifier l'architecture du moteur d'exécution de nouvelle génération `dfdl-vm` (Machine Virtuelle à Bytecode, Compilation JIT et Génération AOT en environnement strict `no_std`), enrichie des **mesures matérielles réelles de profilage CPU** et du benchmark tactique FakeTDL (OpenDFDL).

---

## 1. Contexte & Diagnostic Expérimental des Performances

### 1.1. État Actuel du Moteur Interprété (`dfdl-core`)
Le moteur DFDL interprété actuel fonctionne selon deux phases :
1. **Phase Statique (Compilation XML/XSD) :** `dfdl-schema` compile les schémas DFDL en un graphe IR de termes (`CompiledSchema` avec arène de `CompiledTerm`).
2. **Phase Dynamique (Interprétation à l'Exécution) :** `ParserEngine` et `UnparserEngine` parcourent récursivement ce graphe de termes, évaluent les propriétés de cadrage, résolvent les points d'incertitude (`<choice>`) et construisent un arbre DOM d'objets [`InfosetElement`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs).

### 1.2. Progrès Récents (Optimisations d'Octobre 2026)
Trois optimisations ont été déployées dans `dfdl-core` :
- **Précompilation des Regex & Fast-Path Trivial :** Les motifs discriminants triviaux (`.` / `^.`) sont résolus en $O(1)$ par vérification de fin de flux (`!reader.is_eof()`).
- **Résolution Statique des XPath Relatifs :** Évaluation directe de `{ ../count }` via `find_sibling_in_parent` évitant le clonage d'arbres.
- **Peeking Zero-Copy du BitReader :** Remplacement des boucles de 1 024 lectures par `peek_bytes` / `peek_slice` sans allocation.

**Gain mesuré sur FakeTDLFile (192 octets, 3 messages) :**  
La latence de parsing est passée de **113.33 µs à 60.96 µs** (**-46.2% de latence, 1.86× plus rapide**), réduisant le ratio d'asymétrie parse/unparse de **3.98× (~4:1) à 2.15× (~2:1)**.

---

## 2. Profilage Matériel Réel du CPU (Profilage Hardware ARM64 macOS)

Un profilage matériel à l'échantillonnage (7 825 échantillons, 1 ms, DWARF symbolication complète sur 50 000 itérations de `dfdl-benchmark`) a révélé la décomposition macroscopique exacte du CPU :

### 2.1. Répartition du Temps CPU de la Machine Virtuelle DFDL

| Sous-système | Échantillons CPU | % Temps Total | Fonctions Critiques Identifiées |
|:---|:---:|:---:|:---|
| **1. Allocations & Libérations Heap** | **3 321** | **42.4%** | `_xzm_free` (13.1%), `malloc_zone_malloc` (6.2%), `_free` (5.3%), `_platform_memmove` (5.2%), `__rdl_alloc` (2.2%) |
| **2. Clonages Profonds & Destruction d'Objets** | **1 275** | **16.3%** | `Vec<InfosetNode>::clone` (3.6%), `drop_glue::<InfosetElement>` (3.2%), `InfosetElement::clone` (2.9%), `String::clone` (2.8%) |
| **3. Recherche de Délimiteurs & Chaînes** | **1 031** | **13.2%** | `memchr::memrchr` (2.4%), `_platform_memcmp` (2.3%), `Pattern::is_contained_in` (2.0%) |
| **4. Moteur de Parsing & BitReader** | **685** | **8.8%** | `parse_element_with_seg_internal` (0.9%), `parse_single_element` (0.8%), `BitReader::read_bits` (0.6%) |
| **5. Frais d'Exécution OS / Runtime** | **327** | **4.2%** | `_dyld_start`, horloges haute précision |
| **6. Formatage Nombres & Calendriers** | **123** | **1.6%** | `core::fmt::write`, algorithme Grisu flt2dec |
| **7. Traversée du Graphe de Termes IR** | **75** | **1.0%** | `term_has_representation_impl`, `get_term` |
| **8. Expressions XPath Pest AST** | **71** | **0.9%** | `eval_expr`, `build_expr` |
| **9. Moteur d'Unparsing & BitWriter** | **39** | **0.5%** | `unparse_element_with_seg_internal` (0.8%), `write_bits` (0.5%) |
| **10. Itérateurs Primitifs & Autres** | **878** | **11.2%** | Itération de tranches, déballage d'Option, indexation |

### 2.2. Le Mur des 58.7% de Gestion Mémoire
**Constat capital :** La manipulation des bits et l'application des règles DFDL pures ne représentent que **~10% du temps CPU**. **58.7% du temps CPU** est entièrement consommé par l'allocateur mémoire du système (`malloc`/`free`) et le clonage récursif des nœuds de l'arbre infoset lors des points de choix (`builder.checkpoint()`).

---

## 3. Le Benchmark Comparatif : Interprété vs Codec Natif Rust

L'implémentation dans `dfdl-benchmark` d'un codec natif Rust écrit à la main ([`crates/dfdl-benchmark/src/native.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-benchmark/src/native.rs)) a permis de mesurer le plafond théorique de performance de la machine physique :

| Opération (Message Multi-Trames 192B) | DFDL Interprété | Codec Natif Rust | Facteur d'Accélération |
|:---|:---:|:---:|:---:|
| **Décodage / Parse** | 60.96 µs | **49 ns** | **1 244× plus rapide** |
| **Encodage / Unparse** | 28.35 µs | **39 ns** | **727× plus rapide** |
| **Aller-retour complet (Roundtrip)** | 91.12 µs | **94 ns** | **969× plus rapide** |
| **Débit utile** | 16 238 op/s | **12 320 323 op/s** | **758× supérieur** |
| **Bande passante** | 2.97 Mo/s | **2 255.92 Mo/s (2.25 Go/s)** | **759× supérieur** |

Ce résultat démontre qu'une architecture compilée peut dépasser les **2 Go/s**, contre ~3 Mo/s pour le moteur interprété actuel.

---

## 4. Architecture Cible : Le Crate `dfdl-vm`

Pour franchir le mur des 58.7% et se rapprocher des performances natives sans sacrifier la conformité DFDL v1.0, `dfdl-vm` repose sur trois piliers :

1. **Compilateur de Bytecode Linéaire (ISA Spécialisée DFDL) :** Aplatit l'arbre de termes en une séquence d'opcodes compacts stockés dans un tableau contigu.
2. **Gestionnaire Transactionnel d'Infoset à Zéro Allocation :** Élimination de `builder.checkpoint()`. Le retour arrière lors d'un choix (`choice`) se résume à décrémenter un curseur d'offset dans une arène linéaire (`bump-allocation`).
3. **Double / Triple Backend d'Exécution :**
   - **Interpréteur Bytecode VM (`no_std`, Safe Rust certifiable DO-178C DAL A) :** Latence cible de **3 à 8 µs** (accélération de 8× à 20×).
   - **Micro-JIT (`no_std`, Copy-and-Patch x86_64 / AArch64) :** Latence cible de **200 à 500 ns**.
   - **Générateur AOT (Ahead-Of-Time Codegen) :** Génération de code source Rust compilé statiquement, atteignant les **49 ns** du codec natif.

```
                    ┌─────────────────────────┐
                    │      Schéma DFDL        │
                    └────────────┬────────────┘
                                 │ (dfdl-schema)
                                 ▼
                    ┌─────────────────────────┐
                    │ CompiledSchema (IR)     │
                    └────────────┬────────────┘
                                 │
                 ════════════════╪════════════════════
                                 │ (Bytecode Lowering & Arena Optimizer)
                                 ▼
                    ┌─────────────────────────┐
                    │   dfdl-vm::Bytecode     │
                    │   (Linear BytecodeProg) │
                    └────────────┬────────────┘
                                 │
         ┌───────────────────────┼───────────────────────┐
         ▼                       ▼                       ▼
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Bytecode VM   │     │    no_std JIT   │     │   AOT Rust Gen  │
│   Interpreter   │     │ (Copy-and-Patch)│     │  (Statique)     │
│   (100% Safe)   │     │ (x86_64 / ARM64)│     │ (Zero Overhead) │
│   ~5 µs / trame │     │ ~300 ns / trame │     │ ~49 ns / trame  │
└────────┬────────┘     └────────┬────────┘     └────────┬────────┘
         │                       │                       │
         └───────────────────────┼───────────────────────┘
                                 ▼
                    ┌─────────────────────────┐
                    │  Zero-Alloc Event Sink  │
                    │  ou Arène Contiguë      │
                    └─────────────────────────┘
```

---

## 5. Spécification de l'ISA DFDL-VM (Opcode Set)

L'ISA est conçue pour manipuler directement les registres du bitstream et des valeurs sans passer par des boîtes dynamiques (`DfdlValue`).

### 5.1. Registres Virtuels de la Machine
- `BR` : Pointeur de bitstream courant (`BitReader` / source slice + bit offset).
- `R0` à `R7` : Registres 64 bits d'entiers/flottants généraux.
- `HP` : High-Water Mark de l'arène Infoset (curseur linéaire d'allocation).
- `SP` : Pointeur de pile des points d'incertitude (PoU) et des contextes de délimiteurs.

### 5.2. Opcodes Clés

| Opcode | Opérandes | Fonctionnement & Optimisation Associée |
|---|---|---|
| `READ_BITS_U8` | `rd, n_bits` | Lecture de 1 à 8 bits dans `rd` avec alignement MSBF |
| `READ_BE_U16` | `rd` | Lecture directe de 2 octets en Big-Endian sans appel de fonction |
| `READ_BE_U32` | `rd` | Lecture directe de 4 octets en Big-Endian |
| `READ_BE_F32` | `rd` | Lecture directe d'un flottant IEEE 754 32 bits |
| `CHECK_ALIGN_PAD` | `align_bits` | Avance directe des bits de bourrage (zéro branchement) |
| `PEEK_MATCH_U8` | `expected, offset` | Vérification de délimiteur ou magic sans avancer le pointeur |
| `SAVE_POU` | `dest_ip` | Sauvegarde `(BR, HP)` sur la pile PoU (zéro clone mémoire) |
| `COMMIT_POU` | - | Valide le choix et dépile le PoU |
| `ROLLBACK_POU` | - | Restaure `BR` et `HP = checkpoint_hp` sans libération récursive |
| `JUMP_SWITCH` | `table_id, rd` | Table de saut direct pour les discriminants de choix entiers |
| `EMIT_SCALAR_U32` | `sym_id, rd` | Émet un événement sans allouer de chaîne `QName` |
| `HALT` | `status` | Arrêt du décodage avec code de statut |

---

## 6. Élimination du Mur Mémoire : Le Modèle "Arena & Event Streaming"

Pour éliminer les 58.7% de surcoût d'allocation mesurés dans le profil :
1. **Internement Statique des QNames (`&'schema str`) :**  
   Les identifiants d'éléments ne sont plus stockés dans des `String` heap-allouées, mais sous forme d'index d'arène statique (`SymbolId(u16)`).
2. **Arène Linéaire pour l'Arbre Infoset :**  
   Au lieu de centaines de petits `Vec<InfosetNode>`, tous les nœuds sont alloués consécutivement dans un tampon pré-dimensionné de 64 Ko. Le retour arrière lors d'un choix (`ROLLBACK_POU`) se résume à une simple réaffectation d'index :
   ```rust
   // Zéro allocation, zéro appel à free() !
   self.arena_cursor = checkpoint.arena_cursor;
   self.bit_position = checkpoint.bit_position;
   ```
3. **Sortie Directe par Événements / Serde-like Visitor :**  
   Pour les flux temps réel où la création d'un DOM XML est inutile, la VM pousse directement les champs dans une structure Rust contiguë (comme `FakeTdlFile`) via un trait sans allocation.

---

## 7. Faisabilité du JIT et de l'AOT en `no_std`

### 7.1. Le JIT "Copy-and-Patch"
Plutôt qu'un compilateur JIT lourd (comme Cranelift ou LLVM incompatible avec `no_std`), la technique retenue est le **Copy-and-Patch JIT** :
- De micro-fragments de code assembleur (stubs de 16 à 64 octets pour lire un entier, décaler des bits, vérifier un discriminant) sont précompilés par `rustc` lors du build.
- À l'exécution, lors de la compilation du schéma, le moteur JIT alloue une page RX (via le trait `JitMemoryBackend`) et copie les stubs en injectant les constantes (offsets, masques).
- **Temps de compilation JIT :** Inférieur à **50 microsecondes** pour un schéma comme FakeTDL.

### 7.2. L'Alternative AOT (Génération de Code Statique)
Pour les systèmes critiques soumis aux contraintes avioniques **DO-178C DAL A** où la mémoire W^X dynamique est interdite :
- `dfdl-cli` ou une macro procédurale génère directement le code source Rust équivalent au codec natif à partir du schéma DFDL.
- Ce code est compilé directement avec le binaire de vol, garantissant les **49 ns** de latence et **100% de certifiabilité statique**.

---

## 8. Synthèse Comparative des Solutions d'Exécution

| Critère | DFDL Interprété Actuel | Bytecode VM (`dfdl-vm`) | JIT Copy-and-Patch | AOT Rust Statique |
|:---|:---:|:---:|:---:|:---:|
| **Latence Parse (192B)** | **60.96 µs** | **~4 – 8 µs** | **~300 – 600 ns** | **49 ns** |
| **Débit utile** | **3.0 Mo/s** | **~30 – 60 Mo/s** | **~350 – 700 Mo/s** | **2 255 Mo/s (2.25 Go/s)** |
| **Overhead Mémoire** | 58.7% (malloc/free) | < 2% (arène contiguë) | 0% | 0% |
| **Temps de Setup Schéma** | 18 ms (compilation IR) | +2 ms (génération bytecode) | +0.1 ms (assemblage stubs) | 0 ms (compilation statique) |
| **Compatibilité `no_std`** | 100% | 100% Safe Rust | Requiert trait mémoire RX | 100% Safe Rust |
| **Certifiabilité DO-178C** | Oui | Oui (très haute assurance) | Difficile (RAM exécutable) | **Maximale (DAL A certifiable)** |

---

## 9. Feuille de Route d'Implémentation Recommandée

1. **Jalon 1 : Moteur Transactionnel d'Arène dans `dfdl-core` (Court terme)**
   - Remplacer `builder.checkpoint()` par un journal d'index de tronquage.
   - Éliminer le surcoût de 16.3% de clonage profond mesuré lors du profilage.
2. **Jalon 2 : Crate `dfdl-vm` & Interpréteur Bytecode Pur (Moyen terme)**
   - Définir l'arène contiguë d'instructions et le compilateur `CompiledSchema -> BytecodeProgram`.
   - Fournir l'interpréteur de bytecode 100% Safe Rust `no_std`.
   - Viser le palier de **150 à 300 Mbps** de débit soutenu en environnement embarqué.
3. **Jalon 3 : Backend AOT / Code-Generator (Objectif Performance Ultime)**
   - Développer un générateur de code Rust natif produisant automatiquement un codec similaire à `native.rs` à partir du modèle DFDL.
   - Permettre aux applications temps réel critiques d'atteindre des latences nanosecondes (**< 100 ns**) et un débit gigabit.
