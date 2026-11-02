package com.mitre.gif
import org.junit.Test
import org.apache.daffodil.tdml.Runner

object TestGif {
  val tdmlFile = "com/mitre/gif/gif.tdml"
  val validateTDML = true
  val validateDFDLSchema = true
  lazy val runner = Runner("", tdmlFile)
}

class TestGif {
  import TestGif._

  @Test def test_gif1() = { runner.runOneTest("gif1") }
}
