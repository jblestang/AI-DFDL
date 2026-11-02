/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package com.owlcyberdefense.dfdl

import org.apache.daffodil.lib.exceptions.Assert
import org.apache.daffodil.api.layers.ChecksumLayer
import passera.unsigned.UShort

import java.nio.ByteBuffer
import java.nio.ShortBuffer
import scala.collection.JavaConverters._

/**
 * Computes the IPv4 Packet Checksum as an unsigned short.
 *
 * The resulting checksum is stored into the DFDL variable named IPv4Checksum.
 *
 * There are no other parameter variables for this layer.
 *
 * This layer has a fixed length of 20 bytes. If 20 bytes are not available in
 * the data stream it is a processing error.
 */
final class IPv4ChecksumLayer
  extends ChecksumLayer("IPv4Checksum", "urn:owlcyberdefense.com:schema:dfdl:IPv4Checksum:ipv4cs") {

  private def layerFixedLengthInBytes = 20

  setLength(layerFixedLengthInBytes)

  def getLayerVariableResult_IPv4Checksum: Int = getChecksum

  /**
   * The index (in units of a short, so 2 bytes) of the checksum itself within the
   * layer data
   */
  private def chksumShortIndex = 5

  override def compute(isUnparse: Boolean, bb: ByteBuffer): Int = {
    val shortBuf = bb.asShortBuffer()
    val finalChecksum = computeChecksum(shortBuf)
    if (isUnparse) {
      //
      // clobber the byte buffer bytes corresponding to the checksum with
      // the recomputed value
      //
      shortBuf.put(chksumShortIndex, finalChecksum.toShort)
    }
    finalChecksum
  }

  /**
   * Computes the correct IPv4 checksum from the 20 bytes, skipping the central part that contains the
   * current checksum (which could be incorrect).
   *
   * @return the correct IPv4 checksum, an unsigned 16-bit integer, as an Int
   */
  private def computeChecksum(shortBuf: ShortBuffer): Int = {
    var chksum: Int = 0
    // Notice that there are no loops in this code. It has one conditional branch at the end,
    // but otherwise is straight-line code.
    chksum += UShort(shortBuf.get(0)).toInt
    chksum += UShort(shortBuf.get(1)).toInt
    chksum += UShort(shortBuf.get(2)).toInt
    chksum += UShort(shortBuf.get(3)).toInt
    chksum += UShort(shortBuf.get(4)).toInt
    // chksum += UShort(shortBuf.get(5)).toInt // Don't add this one. This is the incoming existing checksum
    chksum += UShort(shortBuf.get(6)).toInt
    chksum += UShort(shortBuf.get(7)).toInt
    chksum += UShort(shortBuf.get(8)).toInt
    chksum += UShort(shortBuf.get(9)).toInt

    Assert.invariant(chksum >= 0)

    //
    // now combine the carry bits in the most significant 16 bits into the lower 16 bits.
    //
    // Since this is only for the IPv4 header (20 bytes) this can only carry once.
    // Adding the carry into the high 16 bits back to the lower 16 bits can't carry again.
    // Proof: adding 10 maximal 16 bit words, 0xFFFF x 10 = 0x9FFF6.
    // Adding the 0x9 carry to 0xFFF6 is 0xFFFF. No further carry.
    val checksumLow = chksum & 0xffff
    val checksumHigh = chksum >>> 16
    chksum = checksumLow + checksumHigh
    val highPart = chksum >>> 16
    Assert.invariant(highPart == 0) // no further carry is possible.
    val checksumTotalShort = UShort(chksum.toShort)
    val checksum = checksumTotalShort.toInt
    //
    // take ones complement to get the final checksum
    //
    val finalChecksum: Int = (~checksum) & 0xffff
    finalChecksum
  }

}
