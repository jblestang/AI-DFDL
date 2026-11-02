package com.owlcyberdefense.ntp

// Copyright (c) 2012-2026 Owl Cyber Defense. All rights reserved.
// Please see the accompanying COPYRIGHT.txt file for terms of use.

import org.apache.daffodil.tdml.Runner

import org.apache.daffodil.junit.tdml.TdmlSuite
import org.apache.daffodil.junit.tdml.TdmlTests

import org.junit.Test

object TestNtp extends TdmlSuite {
  val tdmlResource = "/com/owlcyberdefense/ntp/TestNtp.tdml"
}

class TestNtp extends TdmlTests {
  val tdmlSuite = TestNtp

  @Test def test_bits_01 = test
  @Test def test_all_ff_01 = test

  @Test def test_data_0 = test

  @Test def test_ntp_li_01 = test
  @Test def test_ntp_li_02 = test
}
