# DFDL Schema for Network Time Protocol (NTP) Messages

This is a DFDL schema for the NTP basic packet format described here:

    https://en.wikipedia.org/wiki/Network_Time_Protocol#NTP_packet_header_format

This schema is written to use the preferred DFDL schema project layout and DFDL schema style.
It can be used as a starting point for other schema projects.

This schema supersedes older NTP schemas from the OpenDFDL github site, with repositories
named:

- ntp-2022-05-19
- ntp2
- ntp1

## License

This schema is licnesed under open-source terms which you can find in the COPYRIGHT.txt file. 

## Testing

There are a few built-in self-tests that are run by way of `sbt test`. 
That also has the side-effect of compiling the DFDL schema to fast-loading binary form.
The pre-compiled DFDL schema lives in `target/dfdl-ntp-1.0.0-daffodil400.bin`, though the 
version number ("1.0.0") of this schema, and the daffodil version number ("400") may change
as this schema evolves. 
Both this schema version number and the daffodil version to use are specified in the `build.sbt`
file. 

There are also a collection of NTP message files extracted from a PCAP capture.
These can be run using the daffodil CLI using this command line (for example):
```bash
daffodil parse --parser target/dfdl-ntp-1.0.0-daffodil400.bin \
  src/test/resources/com/owlcyberdefense/ntp/data/0.dat 
```

# Release Notes

## Version 1.0.0

These aspects of NTP are not implemented:
- optional extension fields past the 48th byte, including the optional checksum
- NTP Control Messages ( described in Appendix B of RFC 1305)
- Kiss-of-Death (KoD) Messages
- refID as choice of 2 different enums
- range checks for the `poll` element and other elements with simple range types (per RFC5905)
