package com.owlcyberdefense.dfdl

import org.apache.daffodil.tdml.Runner
import org.junit.AfterClass
import org.junit.Test

object TestIPAddress {
  lazy val runner = Runner("/com/owlcyberdefense/dfdl/", "testIPAddress.tdml")

  @AfterClass def shutDown(): Unit = {
    runner.reset()
  }
}

class TestIPAddress {

  import TestIPAddress._

  @Test
  def testIPAddress1(): Unit = { runner.runOneTest("ipAddress1") }

}
