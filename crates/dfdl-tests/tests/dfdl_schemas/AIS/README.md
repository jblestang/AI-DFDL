# AIS
Automatic Identification System - for Navigation

A partial DFDL schema for AIS data is part of the Apache Daffodil (https://daffodil.apache.org) project, where it is used as a test of the experimental layering extension to DFDL.

This is not a complete AIS DFDL schema, but it uses the layering extension (proposed for inclusion in the next major version of DFDL) to express the payload armoring algorithm. 

This is the "hard part" of parsing AIS format, the remaining aspects are far less complex.  

See: 
https://github.com/apache/daffodil/blob/main/daffodil-test/src/test/resources/org/apache/daffodil/layers/ais.tdml
and
https://github.com/apache/daffodil/blob/main/daffodil-test/src/test/scala/org/apache/daffodil/layers/AISTransformer.scala

A complete AIS DFDL schema could be created by combining the AIS Payload Armoring example DFDL with additional DFDL schema for the remainder of the fields of the messages. 

