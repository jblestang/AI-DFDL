Syslog Protocol DFDL Schema
====
Includes support for the following syslog variants:

- RFC-5424 (`syslog-rfc5424.dfdl.xsd`)
- RFC-3164 (`syslog-rfc3164.dfdl.xsd`)
- Cisco (`syslog-cisco.dfdl.xsd`)
- ESXi (`syslog-esxi.dfdl.xsd`)

Each file defines the format of a specific variant as well as a root element
that can be used to parse only that variant.

To support all possible variants, the `syslog.dfdl.xsd` file defines a `Syslog`
root element.

To support all possible variants from a Solarwinds log forwarder, which inserts
data immediately after the priority value, the `syslog-solarwinds.dfdl.xsd`
file defines a `SolarwindsSyslog` root element.

## Portability

This schema is known to work with the Daffodil DFDL implementation. It uses
dfdl:inputValueCalc and dfdl:outputValueCalc in the Facility and Severity
elements, which do not work with IBM DFDL.

However, the `common.dfdl.xsd` file can be modified to enable support for IBM
DFDL. See the documentation annotations in the `PriorityValue` groups for
details on the needed adjustments. Additionally, `encodingErrorPolicy` must be
changed from "replace" to "error" when used with IBM DFDL

## Release Notes

### 2.0.1
- Replaces empty sequence in choice to support validation with Xerces-C
- Modifications to make IBM portability testing easier

### 2.0.0
- Modifies Syslog element to support all variants at once
- New schemas support parsing only individual variants
- Removes IBM DFDL portability, but provides details to make it portable

### 1.0.0
- Portable variant that works with IBM DFDL.
- Removed version number from namespace URI.
- Changed branding from Tresys to Owl.

### 0.1.2
- Support for Solarwinds as well as standard Syslog in same schema.

### 0.1.1
- Support for ESXi

### 0.1.0:
- Initial support for RFC-5424, RFC-3164 and Cisco
