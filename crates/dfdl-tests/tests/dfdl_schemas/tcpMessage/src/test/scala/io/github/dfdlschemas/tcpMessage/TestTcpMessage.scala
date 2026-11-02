package io.github.dfdlschemas.tcpMessage

import org.apache.daffodil.tdml.Runner
import org.junit.AfterClass
import org.junit.Test

object TestTcpMessage {
  lazy val runner = Runner("/io/github/dfdlschemas/tcpMessage/", "TestTcpMessage.tdml")

  @AfterClass def shutDown: Unit = {
    runner.reset()
  }
}

class TestTcpMessage {

  import TestTcpMessage._

  @Test def test_tcpMessage_01(): Unit = { runner.runOneTest("test_tcpMessage_01") }
}
