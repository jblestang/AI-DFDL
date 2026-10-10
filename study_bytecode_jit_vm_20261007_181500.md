# Étude Architecturale : Bytecode VM et JIT no_std pour AI-DFDL

**Date :** 2026-10-07 18:15:00 UTC+2  
**Auteur :** Expertise conjointe Architecte Systèmes Rust Bas-Niveau, Compilateurs/VM & Systèmes Critiques (Radar / EW / Cybersécurité)  
**Objectif :** Évaluer la faisabilité, les choix techniques, les contraintes de sécurité et les performances d'un nouveau crate `dfdl-vm` intégrant une Machine Virtuelle à Bytecode et un compilateur JIT en environnement strict `no_std`.

---

## 1. Contexte & Diagnostic des Performances Actuelles

### 1.1. Fonctionnement Actuel de `dfdl-core`
Dans l'architecture existante :
1. **Phase Statique (Compilation de Schéma) :** `dfdl-schema` compile un fichier XML/XSD en un graphe IR de termes (`CompiledSchema` contenant des `CompiledTerm` référencés par `NodeId`).
2. **Phase Dynamique (Parsing de Données) :** Lors de `parse_document()`, le parseur parcourt ce graphe par appels récursifs mutuels (`parse_term_inner` -> `parse_element` / `parse_sequence` / `parse_choice`).

### 1.2. Facteurs Limitants de Performance (Goulots d'Étranglement)
Dans les flux de données temps réel à haut débit (Radar I/Q, Guerre Électronique, Liaisons L16, trames ASTERIX Cat 048/062) :
- **Overhead de Dispatch et Saut Indirect :** À chaque champ, le moteur effectue un lookup dans le vecteur de termes (`schema.get_term(id)`), vérifie le budget (`self.budget.consume(1)`), évalue dynamiquement les alignements, l'ordre des bits (`bitOrder`), et bascule sur un `match` exhaustif.
- **Interprétation Récursive d'Expressions :** Même pour des conditions de longueur simples, l'évaluateur d'expressions XPath/DFDL (`eval_expr`) traverse un AST dynamique issu de Pest.
- **Pression Mémoire et Cache L1/L2 :** La structure `CompiledSchema` et `ResolvedProperties` est dispersée en mémoire. L'état d'exécution (`ParserState`) maintient de nombreux vecteurs et piles d'état (`in_scope_delimiters`, `validation_errors`, `enclosing_complex_elements`), provoquant des cache-misses constants.
- **Émission d'Infoset Granulaire :** Chaque petit champ génère un événement `InfosetEvent::SimpleValue` avec clonage de `QName` et allocation d'arbres.

---

## 2. Architecture Cible : Le Crate `dfdl-vm`

L'approche optimale repose sur une séparation claire entre :
1. **Frontend / Schéma :** `dfdl-schema` continue de valider et produire le `CompiledSchema`.
2. **Middle-end (Optimiseur & Générateur de Bytecode) :** Traduction de `CompiledSchema` en un flux d'instructions linéaire (`DfdlBytecodeProgram`), exempt de récursivité et optimisé (dead-branch elimination, bit-offset folding, alignment coalescing).
3. **Backend d'Exécution :** Deux moteurs d'exécution interchangeables :
   - **Interpréteur Bytecode VM (`no_std`, 100% Safe Rust) :** Universel, déterministe, hautement portable (du microcontrôleur Cortex-M au serveur multicœur).
   - **Moteur JIT (`no_std`, architectural) :** Génération directe d'instructions machine natives pour x86_64 et AArch64 pour les environnements autorisant l'exécution dynamique de code.

```
                    ┌─────────────────────────┐
                    │      Schéma DFDL        │
                    └────────────┬────────────┘
                                 │ (Compilation XML/XSD)
                                 ▼
                    ┌─────────────────────────┐
                    │ CompiledSchema (IR)     │
                    └────────────┬────────────┘
                                 │
                 ════════════════╪════════════════════
                                 │ (Bytecode Lowering & Optimizations)
                                 ▼
                    ┌─────────────────────────┐
                    │   dfdl-vm::Bytecode     │
                    │      (BytecodeProgram)  │
                    └────────────┬────────────┘
                                 │
                   ┌─────────────┴─────────────┐
                   ▼                           ▼
        ┌─────────────────────┐     ┌─────────────────────┐
        │   Bytecode VM       │     │    no_std JIT       │
        │   Interpreter       │     │    (x86_64 / ARM64) │
        │ (100% Safe no_std)  │     │ (Executable Memory) │
        └──────────┬──────────┘     └──────────┬──────────┘
                   │                           │
                   └─────────────┬─────────────┘
                                 ▼
                    ┌─────────────────────────┐
                    │   Infoset / Événements  │
                    └─────────────────────────┘
```

---

## 3. Conception de l'ISA Bytecode Spécialisée (DFDL-ISA)

Contrairement à une VM générique (comme JVM ou WASM), l'ISA doit être ultra-spécialisée pour le décodage de bitstream.

### 3.1. Jeu d'Instructions (Exemples d'Opcodes)

| Opcode | Opérandes | Description & Sémantique |
|---|---|---|
| `READ_BITS_U8` | `reg_dest, n_bits` | Lit 1 à 8 bits non signés dans le registre |
| `READ_BITS_U16_BE` | `reg_dest` | Lit 16 bits en Big-Endian (fusionne lecture + bswap) |
| `READ_BITS_U32_BE` | `reg_dest` | Lit 32 bits en Big-Endian |
| `READ_BITS_U64_BE` | `reg_dest` | Lit 64 bits en Big-Endian |
| `READ_VAR_INT` | `reg_dest, length_reg` | Lit un entier de longueur dynamique spécifiée par un registre |
| `ALIGN_MANDATORY` | `alignment_bits` | Alignement statique direct ($0$ à $align-1$ bits sautés) |
| `SKIP_BITS` | `n_bits` | Avance de $n$ bits dans le bitstream sans allouer |
| `MATCH_MAGIC` | `const_pool_idx` | Vérifie immédiatement des octets magiques (ex: En-tête de trame) |
| `JUMP_IF_EQ` | `reg, imm, offset` | Branchement conditionnel (gestion des `choice` / discriminators) |
| `ENTER_ELEMENT` | `elem_id` | Émet un début d'élément dans l'Infoset |
| `LEAVE_ELEMENT` | `elem_id` | Émet une fin d'élément |
| `EMIT_VALUE_INT` | `elem_id, reg` | Émet un entier simple sans boxing intermédiaire |
| `PUSH_POU` | `backtrack_offset` | Établit un point d'incertitude (sauvegarde position bitstream) |
| `POP_POU` | - | Valide le chemin et retire le point d'incertitude |
| `HALT` | `status` | Fin du décodage avec succès ou code d'erreur |

### 3.2. Optimisations Statiques lors de la Génération de Bytecode
Une fois le schéma parsé, de nombreuses propriétés sont **constantes et statiques** :
1. **Fusion d'Alignements et de Sauts (Framing Coalescing) :** Si un élément requiert un `leadingSkip` de 8 bits suivi d'un alignement sur 32 bits, le compilateur calcule la position statique et émet une seule instruction de saut compacte.
2. **Fast-Path pour Chiffres Fixes Alignés :** Si un bloc de 4 entiers de 16 bits Big-Endian est consécutif et aligné sur un octet, le compilateur remplace 4 lectures de bits avec gestion de frontière par une seule lecture vectorielle native ou un cast direct mémoire.
3. **Discriminator Flattening :** Transformation d'une hiérarchie complexe de `xs:choice` en une table de saut directe (Jump Table) indexée par la valeur du champ discriminant.

---

## 4. Faisabilité du JIT en Rust `no_std`

### 4.1. Le Défi de `no_std` pour la Génération de Code Natif
La génération de code machine à l'exécution (JIT) requiert fondamentalement :
1. **L'allocation de mémoire avec permissions d'exécution (RX).**
2. **Le respect du principe W^X (Write XOR Execute) :** la page mémoire doit être en écriture (RW) pendant l'assemblage des octets machine, puis basculée en lecture/exécution (RX) avant le saut.
3. **La cohérence des caches d'instructions et de données :** Sur ARM/AArch64 et RISC-V (architectures à caches I/D séparés non cohérents), il faut invalider le cache d'instructions (`ISB`, `DSB`, `clear_cache`).

En Rust standard (`std`), cela est pris en charge par `libc::mprotect` ou l'API Windows `VirtualProtect`. En environnement `no_std` :
- Il n'y a **aucun accès direct aux appels système OS**.
- Le compilateur Rust natif n'embarque pas d'allocateur exécutable `no_std`.

### 4.2. Solutions Techniques Viables

#### Option A : Abstraction par Trait Mémoire (`JitMemoryBackend`)
Pour que le crate reste strictement `no_std`, l'allocation de pages exécutables doit être déléguée à l'environnement hôte via un trait :

```rust
pub trait JitMemoryBackend {
    type Error;
    /// Alloue un bloc mémoire accessible en écriture
    fn allocate_rw(&mut self, size: usize) -> Result<*mut u8, Self::Error>;
    /// Bascule la mémoire en mode exécutable (W^X) et purge les caches I/D
    fn make_executable(&mut self, ptr: *mut u8, size: usize) -> Result<*const u8, Self::Error>;
    /// Libère la mémoire allouée
    fn free(&mut self, ptr: *const u8, size: usize);
}
```

- **Sur Linux / macOS (développement, edge servers) :** Une implémentation optionnelle (activable via `cfg(feature = "std")` ou un crate d'intégration) appelle `mmap` / `mprotect`.
- **Sur RTOS / Bare-Metal (FreeRTOS, seL4, Cortex-R/A) :** Le BSP (Board Support Package) fournit une région de RAM configurée en RX dans la MPU/MMU et exécute les instructions de barrière de synchronisation de cache.

#### Option B : Le Choix du Moteur de Génération de Code (Codegen)
Pourquoi **Cranelift** ou **LLVM** sont exclus :
- Cranelift et LLVM dépendent lourdement de `std`, allouent des dizaines de mégaoctets de structures de données intermédiaires et ont un temps de compilation incompatible avec les systèmes embarqués contraints.

Les alternatives retenues :
1. **Micro-Émetteur d'Opcode Direct (Hand-crafted Macro-Assembler) :**
   - Étant donné que le décodage DFDL binaire ne nécessite qu'un sous-ensemble minuscule de x86_64 et AArch64 (instructions de chargement, décalage binaire, masquage `AND`, comparaison et saut), un émetteur d'octets direct de moins de 1000 lignes de code en `no_std` suffit amplement !
   - Zéro dépendance externe.
   - Temps de compilation JIT quasi instantané (inférieur à la microseconde pour un schéma typique).
2. **Technique du "Copy-and-Patch JIT" :**
   - Modèle ultra-moderne (popularisé par Peter Shen et utilisé dans les runtimes légers récents) : de petits fragments de code natif (stubs) sont précompilés par `rustc` en objets statiques. Le moteur JIT se contente de copier ces stubs et d'injecter les constantes (offsets, masques).
   - Performance de code proche de LLVM, vitesse de JIT 100x plus rapide, compatible `no_std`.

---

## 5. Analyse de Sécurité & Conformité Militaire / Critique

Dans les domaines Radar, EW (Guerre Électronique) et Défense, la possibilité d'utiliser un JIT soulève des exigences strictes :

### 5.1. Risques Cyber du JIT
- **JIT Spraying & Exécution Arbitraire :** Si un attaquant injecte un schéma DFDL malveillant visant à forcer la génération de gadgets d'exploitation dans la mémoire exécutable.
  - *Mitigation :* Le bytecode doit être validé par un **Vérificateur Formel (Bytecode Verifier)** avant toute tentative de JIT. Aucun branchement vers des adresses absolues arbitraires n'est permis ; tous les sauts sont relatifs et bornés dans le segment de code.
- **Violation de W^X :** La mémoire ne doit jamais être simultanément inscriptible et exécutable. En cas de non-respect, une vulnérabilité de corruption mémoire externe pourrait écraser le code compilé.
- **Certification Avionique / Sécurité DO-178C :** Dans de nombreux systèmes de défense embarqués (aéronefs, missiles, satellites), l'exécution de code généré dynamiquement en RAM est **formellement interdite** par les autorités de certification.
  - *Conséquence indispensable :* Le système **DOIT** être utilisable avec l'Interpréteur Bytecode pur sans activer le JIT, voire supporter la compilation **AOT (Ahead-of-Time)** via génération de code Rust statique au moment de la compilation.

---

## 6. Comparatif de Performance Attendu

| Caractéristique | Interpréteur Actuel (`dfdl-core`) | Bytecode VM (`dfdl-vm`) | JIT Native (`dfdl-vm` + JIT) |
|---|---|---|---|
| **Modèle d'exécution** | Arbre récursif de termes | Boucle de dispatch linéaire | Code natif x86_64 / AArch64 direct |
| **Overhead par champ** | ~50 à 150 ns (lookups, checks) | ~5 à 15 ns (1 cycle de dispatch) | ~0.5 à 2 ns (instructions machine pures) |
| **Compatibilité `no_std`** | 100% | 100% | Nécessite un hook mémoire RX (MPU/mprotect) |
| **Débit estimé (ASTERIX / Radar)** | ~10 - 30 Mo/s | ~150 - 400 Mo/s | ~1 - 3 Go/s (proche de la vitesse mémoire) |
| **Empreinte binaire** | ~100 Ko | +30 Ko | +50 Ko (micro-assembleur) |
| **Sécurité mémoire / Certification** | Conforme Safe Rust | Conforme Safe Rust / Certifiable | Soumis à validation W^X / Non certifiable DO-178C DAL A |

---

## 7. Structure Recommandée du Crate `dfdl-vm`

```
crates/dfdl-vm/
├── Cargo.toml                   # [features] default = ["alloc"], jit = []
├── src/
│   ├── lib.rs                   # Interface publique (Decoder, Program, Engine)
│   ├── isa.rs                   # Opcode, Register, Instruction, BytecodeProgram
│   ├── compiler/
│   │   ├── mod.rs               # Abaissement de CompiledSchema -> BytecodeProgram
│   │   ├── optimizer.rs         # Coalescing de bits, élimination de redondance
│   │   └── verifier.rs          # Vérificateur statique de sécurité (bornes, budgets)
│   ├── interpreter/
│   │   ├── mod.rs               # Interpréteur no_std déterministe
│   │   └── state.rs             # Registres virtuels, pile de retour, checkpoints
│   └── jit/                     # Activé uniquement si cfg(feature = "jit")
│       ├── mod.rs               # Façade JIT
│       ├── memory.rs            # Trait JitMemoryBackend & W^X guard
│       ├── emitter_x86_64.rs    # Micro-assembleur x86-64 no_std
│       ├── emitter_aarch64.rs   # Micro-assembleur AArch64 no_std
│       └── trampoline.rs        # Interface d'appel native sécurisée
```

---

## 8. Recommandation Stratégique & Feuille de Route

1. **Phase 1 : Crate `dfdl-vm` avec Interpréteur Bytecode Pur (`no_std`)**
   - Définir l'ISA DFDL optimisée pour les formats binaires.
   - Implémenter le compilateur `CompiledSchema -> BytecodeProgram`.
   - Fournir l'interpréteur déterministe `no_std`, 100% Safe Rust, respectant les lints stricts du workspace (`deny(panic, unwrap, indexing_slicing)`).
   - **Gain attendu immédiat : x5 à x15 en débit de parsing**, avec zéro risque cyber et portabilité totale.

2. **Phase 2 : Optimisations du Bytecode & Vérificateur Statique**
   - Mise en place du `BytecodeVerifier` pour garantir l'absence de dépassement de pile, de boucle infinie et de lecture hors borne.
   - Pliage des constantes de framing (alignement, leadingSkip, simple big-endian fast-path).

3. **Phase 3 : Backend JIT Optionnel (`feature = "jit"`)**
   - Spécifier le trait `JitMemoryBackend` pour préserver l'indépendance de `no_std`.
   - Écrire un micro-émetteur AArch64 / x86_64 minimaliste pour les opcodes de lecture de champs binaires critiques.
   - Basculer en exécution JIT les segments de schéma sans incertitude (fast-path natif), tout en conservant l'interpréteur VM en fallback pour les expressions dynamiques ou le backtracking complexe.
