package com.owlcyberdefense.gpssps

import org.junit.AfterClass
import org.junit.Test

import org.apache.daffodil.tdml.Runner

object TestGpssps {
  lazy val runner = Runner("/com/owlcyberdefense/gpssps/", "TestGpssps.tdml")

  @AfterClass def shutDown {
    runner.reset
  }
}

class TestGpssps {

  import TestGpssps._

  @Test def test_gpssps_01(): Unit = {
    runner
      // .runOneTest("test_gpssps_01")
  }
}
