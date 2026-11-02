# Revue d'Expertise DFDL : `dfdl-xml`

- **Date :** 2026-10-28
- **Horodatage :** 17:47:26 UTC+1
- **Cible :** `crates/dfdl-xml` (`lib.rs`, `reader.rs`, `event.rs`, `limits.rs`)
- **Référence Standard :** OGF GFD-P-R.207 DFDL v1.0 (Section 4, 5, 6, 23)
- **Auteur :** Expert Domaine DFDL (Data Format Description Language)

---

## 1. Contexte Normatif & Rôle dans la Pile DFDL

Dans l'écosystème DFDL (GFD-P-R.207) :
1. **La grammaire du schéma (§4.1) :** Les schémas DFDL sont formellement des documents XML Schema 1.0 (W3C XSD) enrichis d'annotations sémantiques DFDL (`dfdl:format`, `dfdl:element`, `dfdl:sequence`, etc.). La fidélité du parser XML conditionne directement l'exactitude de la compilation du schéma.
2. **L'Information Set DFDL (§4.2) :** L'infoset produit lors du parsing ou consommé lors de l'unparsing est une projection de l'arbre XML Infoset (éléments simples, éléments complexes, attributs spéciaux `xsi:nil`, etc.).
3. **Le langage d'expression DFDL (§23) :** Les expressions DFDL (sous-ensemble XPath 2.0) sont stockées dans les valeurs d'attributs XML ou dans des blocs `<dfdl:assert><![CDATA[{ ... }]]></dfdl:assert>`.

---

## 2. Analyse de Conformité au Standard DFDL v1.0

### 2.1. Résolution des Espaces de Noms (Namespaces in XML)
- **Conformité observée :**
  - La gestion de la pile de namespaces dans [`NamespaceFrame`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-xml/src/reader.rs#L36-L42) implémente fidèlement les portées lexicales XML 1.0 :
    - Héritage de l'espace de noms par défaut (`xmlns="..."`) vers les éléments enfants.
    - Résolution des préfixes (`xmlns:xs="..."`, `xmlns:dfdl="..."`) avec masquage local (shadowing).
    - Mécanisme d'injection des bindings initiaux via `add_namespace_binding()` indispensable pour les tests TDML où les espaces de noms sont souvent déclarés au niveau du document racine `<tdml:testSuite>`.
  - Mode permissif vs strict : `set_permissive_namespaces(bool)` permet d'adapter la rigueur selon que le schéma est compilé de manière autonome ou extrait d'un flux composite.
- **Écart / Limitation DFDL :**
  - En DFDL v1.0 §5.2, plusieurs URI de namespaces sont admises pour le vocabulaire DFDL :
    - `http://www.ogf.org/dfdl/dfdl-1.0/` (URI canonique officielle GFD-P-R.207)
    - `http://www.ogf.org/dfdl/` (forme raccourcie acceptée par Daffodil)
    - `http://www.dfdl.org/7793` (utilisé dans de nombreux schémas historiques)
  - `dfdl-xml` reste neutre vis-à-vis des URIs (ce qui est correct architecturalement), mais la transmission de l'URI exacte et de son préfixe vers `dfdl-schema` doit préserver l'identité sémantique des attributs DFDL.

### 2.2. Blocs CDATA et Expressions XPath Multi-lignes (§23)
- **Gestion des événements CDATA :**
  - `XmlReader` émet un `XmlEvent::CData { content, location }`.
  - Dans les schémas DFDL réels (notamment complexes ou contenant des expressions XPath contenant des chevrons `<`, `>`, ou `&`), les utilisateurs encapsulent le code dans `<![CDATA[ ... ]]>`.
  - **Constat :** Dans `dfdl-xml`, le texte CDATA n'est pas normalisé (conformément à XML 1.0), préservant les espaces et sauts de ligne nécessaires à la lisibilité des expressions.
  - **Point de vigilance :** L'interaction entre les événements `Text` et `CData` consécutifs (par exemple `<dfdl:assert>prefix <![CDATA[expr]]> suffix</dfdl:assert>`) doit être correctement concaténée lors de la compilation dans `dfdl-schema`.

### 2.3. Sécurité contre les Attaques XML (Anti-XXE & DTD Prohibition)
- **Conformité de sécurité :**
  - Le standard DFDL (§4.1) restreint l'usage des DTDs pour prévenir les vulnérabilités de type XML External Entity (XXE) et déni de service ("Billion Laughs").
  - `XmlReader` rejette strictement tout `Token::DtdStart` ou `Token::EntityDeclaration` en retournant une `SchemaDefinitionError` explicite ([`reader.rs:698-704`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-xml/src/reader.rs#L698-L704)).
  - Cette protection est parfaitement alignée avec les exigences de sécurité militaire et cyber-défense imposées aux moteurs de parsing de formats binaires.

### 2.4. Absence de Sérialiseur Symétrique (`XmlWriter`)
- **Écart fonctionnel majeur :**
  - DFDL v1.0 traite de manière symétrique le parsing (Données brutes $\rightarrow$ Infoset XML) et l'unparsing (Infoset XML $\rightarrow$ Données brutes).
  - Actuellement, `dfdl-xml` est uniquement un **lecteur** (`XmlReader`).
  - L'écriture d'un arbre Infoset en XML (dans `dfdl-core::infoset::tree::to_xml()`) est effectuée de manière ad-hoc par concaténation de chaînes dans `dfdl-core`.
  - **Recommandation DFDL :** Fournir dans `dfdl-xml` un composant `XmlWriter` / `XmlEventWriter` garantissant l'échappement rigoureux des caractères réservés (`&`, `<`, `>`, `"`) et la déclaration canonique des namespaces lors de l'export d'infosets.

---

## 3. Matrice de Conformité DFDL / XML

| Fonctionnalité XML requise par DFDL | Statut | Commentaire DFDL |
|:---|:---:|:---|
| Parsing éléments & attributs XSD | **Conforme** | Tokenisation robuste et sûre via `xmlparser`. |
| Résolution des préfixes `xmlns` | **Conforme** | Support des contextes imbriqués et injection externe. |
| Préservation des expressions CDATA | **Conforme** | Événement dédié `XmlEvent::CData` sans altération des chaînes. |
| Entités XML standard (`&amp;`, `&lt;`, etc.) | **Conforme** | Décodage in-place dans les valeurs textuelles et attributs. |
| Rejet strict des DTDs (Anti-XXE) | **Conforme** | Conforme aux directives de sécurité DFDL / NIST / ANSSI. |
| Sérialisation XML Infoset (`XmlWriter`) | **Non implémenté** | Délégué de façon non unifiée à `dfdl-core`. |

---

## 4. Conclusion de l'Expert DFDL
`dfdl-xml` remplit avec précision son rôle d'ingestion de schémas XSD DFDL. La robustesse face aux spécifications XML 1.0 et la politique de sécurité stricte (anti-XXE) constituent un socle fiable pour le compilateur de schéma. Le principal manque du point de vue de la chaîne d'outils DFDL réside dans l'absence d'un sérialiseur XML symétrique pour la génération d'infosets conformes.
