Ethernet, IP, TCP, UDP, ICMP, DNS
====

DFDL Schemas for Ethernet and IP

It works with Daffodil (starting from Daffodil version 3.2.0), 
but not with IBM DFDL (as of 2025-10-21) as the latter does not yet support
the dfdl:inputValueCalc, dfdl:outputValueCalc, 
dfdl:hiddenGroupRef, dfdl:valueLength(), dfdl:contentLength(),
and dfdl:newVariableInstance DFDL capabilities.

As of version 1.1.0 of this DFDL schema, it also uses a Daffodil-specific DFDL extension called layers in order
to compute IPv4 checksums as part of parsing and unparsing. 

This schema was created based on the corresponding DFDL types in the PCAP schema, which has been refactored
to use this schema as a component dependency.

This division into two schemas is intended to illustrate how to structure DFDL schemas as 
separate DFDL component
schemas that are assembled together to form a complete format, and which involve layering where a 
container/envelope/header structure, contains a payload structure. PCAP is a container format for Ethernet/IP packets. 

## Usage

The main schema file is `ethernetIP`. It defines the `Ethernet` complex type in the
target namespace. Assuming version 1.5.0 of this schema, using this type requires this namespace 
prefix definition:

        xmlns:eth="urn:owlcyberdefense.com:schema:dfdl:ethernet:eth"

and this import and default format top level annotation:

    <import namespace="urn:owlcyberdefense.com:schema:dfdl:ethernet:eth"
               schemaLocation="/com/owlcyberdefense/dfdl/xsd/ethernetIP.dfdl.xsd"/>

    <annotation>
      <appinfo source="http://www.ogf.org/dfdl/">
        <dfdl:format ref="eth:basicByteBinary"/> 
      </appinfo>
    </annotation>

after which an element can be defined that contains an ethernet packet as:

    <element name="ethernet" type="eth:Ethernet"/>


## Release Notes

### 1.5.0

Namespace URI for PCAP now updated to `urn:owlcyberdefense.com:schema:dfdl:ethernet:eth`,
following the DFDL Schema Style Guide. 

Other internal namespaces for bb, ipa, removed.

### 1.4.2

Conforms to current (2025-10-17) DFDL schema style guidance. 
Many people use this DFDL schema as part of learning about DFDL, so it 
needed to be updated to reflect known best practices.

### 1.4.1
Updated for Daffodil version 4.0.0

### 1.4.0
Updated for Daffodil version 3.8.0
Uses Daffodil new layer API for IPv4 checksum

### 1.3.0
Updated for Daffodil version 3.5.0

### 1.2.0
Updated for Daffodil version 3.4.0

This schema is used as a component schema by the PCAP DFDL schema. 
Hence, a Daffodil 3.4.0 version of this schema is needed.

### 1.1.0
As of version 1.1.0 of this DFDL schema, it also uses a Daffodil-specific DFDL extension called layers in order
  to compute IPv4 checksums as part of parsing and unparsing.

Works with Daffoddil 3.2.1 
