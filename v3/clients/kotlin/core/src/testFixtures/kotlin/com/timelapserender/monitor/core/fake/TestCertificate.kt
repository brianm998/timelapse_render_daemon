// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.fake

import com.timelapserender.monitor.core.net.certificateFingerprintSha256Hex
import java.nio.file.Files
import java.nio.file.Path
import java.security.KeyStore
import java.security.PrivateKey
import java.security.cert.X509Certificate
import java.util.Base64
import java.util.concurrent.TimeUnit

/** A self-signed certificate generated on the spot with the JDK's own keytool, in the system temp dir. */
class TestCertificate private constructor(
    val certificate: X509Certificate,
    private val privateKey: PrivateKey,
) {
    val fingerprintSha256Hex: String = certificateFingerprintSha256Hex(certificate)

    val certificatePem: String get() = pem("CERTIFICATE", certificate.encoded)
    val privateKeyPem: String get() = pem("PRIVATE KEY", privateKey.encoded)

    private fun pem(label: String, der: ByteArray): String =
        "-----BEGIN $label-----\n" +
            Base64.getMimeEncoder(64, "\n".toByteArray()).encodeToString(der) +
            "\n-----END $label-----\n"

    companion object {
        fun generate(): TestCertificate {
            val dir: Path = Files.createTempDirectory("trm-test-cert")
            try {
                val store = dir.resolve("store.p12")
                val exe = if (System.getProperty("os.name").lowercase().contains("win")) "keytool.exe" else "keytool"
                val keytool = Path.of(System.getProperty("java.home"), "bin", exe).toString()
                val process = ProcessBuilder(
                    keytool, "-genkeypair", "-alias", "trd", "-keyalg", "RSA", "-keysize", "2048",
                    "-dname", "CN=trd-test", "-validity", "2", "-storetype", "PKCS12",
                    "-keystore", store.toString(), "-storepass", PASSWORD, "-keypass", PASSWORD,
                ).redirectErrorStream(true).start()
                val output = process.inputStream.readBytes().toString(Charsets.UTF_8)
                check(process.waitFor(120, TimeUnit.SECONDS) && process.exitValue() == 0) { "keytool failed: $output" }
                val ks = KeyStore.getInstance("PKCS12")
                Files.newInputStream(store).use { ks.load(it, PASSWORD.toCharArray()) }
                return TestCertificate(
                    ks.getCertificate("trd") as X509Certificate,
                    ks.getKey("trd", PASSWORD.toCharArray()) as PrivateKey,
                )
            } finally {
                Files.walk(dir).sorted(Comparator.reverseOrder()).forEach(Files::delete)
            }
        }

        private const val PASSWORD = "changeit"
    }
}
