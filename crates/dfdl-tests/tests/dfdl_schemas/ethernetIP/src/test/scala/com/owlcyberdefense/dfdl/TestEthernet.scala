package com.owlcyberdefense.dfdl

import org.apache.daffodil.tdml.Runner
import org.junit.AfterClass
import org.junit.Test

object TestEthernet {
  lazy val runner = Runner("/com/owlcyberdefense/dfdl/", "testEthernet.tdml")

  @AfterClass def shutDown(): Unit = {
    runner.reset()
  }
}

class TestEthernet {

  import TestEthernet._

  @Test def test_icmp1(): Unit = { runner.runOneTest("icmp1") }
  @Test def test_icmp_array(): Unit = { runner.runOneTest("icmp_array") }

  @Test def test_icmp1u(): Unit = { runner.runOneTest("icmp1u") }
  @Test def test_icmp2_bad_checksum(): Unit = { runner.runOneTest("icmp2_bad_checksum") }

  @Test def test_dns1(): Unit = { runner.runOneTest("dns1") }
  @Test def test_http_ipv6_1(): Unit = { runner.runOneTest("http_ipv6_1") }
  @Test def test_tcp_ecn_1(): Unit = { runner.runOneTest("tcp_ecn_1") }

}
