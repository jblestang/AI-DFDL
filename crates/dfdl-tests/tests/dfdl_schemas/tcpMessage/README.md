## DFDL Schema: tcpMessage - simple length header for messages on byte streams

TCP streams have no notion of datagrams.

This schema provides the minimal notion of a TCP datagram, just a length field 
(32-bits, on a byte boundary, big-endian) followed by data of that length in bytes.

The length field's value is in addition to the 4 bytes of the length field itself. 
For example if a message has 100 bytes of data. The length field will contain 
the value 100, and the total number of bytes on the data stream, 
including the length field will be 104 bytes. 

## Usage

This is a _component_ DFDL schema intended to be used by reference from an _assembly_ schema
that incorporates this header and the payload/content after it.

Herein when we refer to _your schema_ we mean the assembly schema that is defining 
messages that use this TCP message component schema to get a TCP datagram.  

To use this schema, the assembly schema must contain a file 
similar to the 
`com/example/xsd/message.dfdl.xsd` file (that you will find 
under `src/test/resources/xsd/`)
which defines the root element of the TCP message.
In your schema you would replace the `com/example` part of the
path name with something more suitable. 

> **TIP**
> In your DFDL schema, the files described here will not be under `src/test/resources/xsd`,
> but rather `src/main/resources/xsd`. They are test files in this schema but main files in
> yours. 

In addition the assembly schema must contain a file like the
`io/github/dfdlschemas/tcpMessage/xsd/tcpContentType.dfdl.xsd` (that you
will find under `src/test/resources/xsd/`) which must be located in this
exact directory location.

The maximum message length defaults to 65535, but can be adjusted upward or downward
by editing the default value of the DFDL variable `tcpHdr:maxLength` (in the `tcpContentType.dfdl.xsd` file)
to the maximum length of your messages
not including the 4 bytes of the stored length field.

Finally, the assembly schema must contain a file similar to the 
`com/example/xsd/payloadGroup.dfdl.xsd` which defines the fields of 
your messages. You would replace the `com/example` part of this 
directory path with more suitable names. 

> **TIP**:
> An available example _assembly_ DFDL schema which uses this tcpMessage
> _component_ DFDL schema is the
> [envelope-payload DFDL schema](
> https://github.com/DFDLSchemas/envelope-payload
> ).
