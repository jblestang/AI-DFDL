/* Copyright (c) 2017-2020 Owl Cyber Defense. All rights reserved.
 *
 * Developed by: Owl Cyber Defense (formerly Tresys Technology), LLC
 *               http://www.owlcyberdefense.com
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of
 * this software and associated documentation files (the "Software"), to deal with
 * the Software without restriction, including without limitation the rights to
 * use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
 * of the Software, and to permit persons to whom the Software is furnished to do
 * so, subject to the following conditions:
 *
 *  1. Redistributions of source code must retain the above copyright notice,
 *     this list of conditions and the following disclaimers.
 *
 *  2. Redistributions in binary form must reproduce the above copyright
 *     notice, this list of conditions and the following disclaimers in the
 *     documentation and/or other materials provided with the distribution.
 *
 *  3. Neither the names of Owl Cyber Defense, nor the names of its contributors
 *     may be used to endorse or promote products derived from this Software
 *     without specific prior written permission.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * CONTRIBUTORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS WITH THE
 * SOFTWARE.
 */

package com.owlcyberdefense.syslog

import org.junit.Test
import org.apache.daffodil.tdml.Runner
import org.junit.AfterClass

object TestSyslog {
  lazy val runner = Runner("/com/owlcyberdefense/syslog/", "testsSyslog.tdml")

  @AfterClass def shutDown = {
    runner.reset()
  }
}

class TestSyslog {

  import TestSyslog._

  @Test def test_syslog_5424_example_01() = { runner.runOneTest("syslog_5424_example_01") }
  @Test def test_syslog_5424_example_02() = { runner.runOneTest("syslog_5424_example_02") }
  @Test def test_syslog_5424_example_03() = { runner.runOneTest("syslog_5424_example_03") }
  @Test def test_syslog_5424_example_04() = { runner.runOneTest("syslog_5424_example_04") }
  @Test def test_syslog_5424_timestamp_no_nano() = { runner.runOneTest("syslog_5424_timestamp_no_nano") }
  @Test def test_syslog_5424_invalid_hostname_char() = { runner.runOneTest("syslog_5424_invalid_hostname_char") }
  @Test def test_syslog_5424_invalid_msgid_length() = { runner.runOneTest("syslog_5424_invalid_msgid_length") }
  @Test def test_syslog_5424_invalid_prival() = { runner.runOneTest("syslog_5424_invalid_prival") }
  @Test def test_syslog_5424_invalid_sdid() = { runner.runOneTest("syslog_5424_invalid_sdid") }
  @Test def test_syslog_5424_missing_hostname() = { runner.runOneTest("syslog_5424_missing_hostname") }

  @Test def test_syslog_3164_example_01() = { runner.runOneTest("syslog_3164_example_01") }
  @Test def test_syslog_3164_example_02() = { runner.runOneTest("syslog_3164_example_02") }
  @Test def test_syslog_3164_failing_messages_nopad_timestamp() = { runner.runOneTest("syslog_3164_failing_messages_nopad_timestamp") }
  @Test def test_syslog_3164_failing_messages_zeropad_timestamp() = { runner.runOneTest("syslog_3164_failing_messages_zeropad_timestamp") }

  @Test def test_syslog_cisco_example_01() = { runner.runOneTest("syslog_cisco_example_01") }

  @Test def test_syslog_esxi_example_01() = { runner.runOneTest("syslog_esxi_example_01") }
  @Test def test_syslog_esxi_example_02() = { runner.runOneTest("syslog_esxi_example_02") }
  @Test def test_syslog_esxi_missing_timestamp_info() = { runner.runOneTest("syslog_esxi_missing_timestamp_info") }
  @Test def test_syslog_esxi_missing_message() = { runner.runOneTest("syslog_esxi_missing_message") }

}
