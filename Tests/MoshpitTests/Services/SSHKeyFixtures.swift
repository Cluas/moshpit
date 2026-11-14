import Citadel
import Foundation
import Testing
@testable import Moshpit

/// Real private keys in each form people paste: what `ssh-keygen` writes by
/// default, with `-m PEM` and with `-m PKCS8`, each with and without a
/// passphrase, plus OpenSSL's PKCS #8 Ed25519 (which macOS's `ssh-keygen`
/// won't write). Made for these tests, used nowhere else, and never near a
/// network.
///
/// `publicKey` and `fingerprint` are `ssh-keygen`'s own: the `.pub` it wrote
/// beside each key and `ssh-keygen -lf` over it (for the OpenSSL key, the
/// public key `openssl pkey -pubout` gives, framed by hand). Nothing here was
/// derived by the code under test.
enum SSHKeyFixtures {
    static let passphrase = "correct horse battery"

    struct Key: Sendable, CustomTestStringConvertible {
        let name: String
        let privateKey: String
        let isEncrypted: Bool
        /// The `.pub` line, comment included.
        let publicKey: String
        let fingerprint: String
        let algorithm: SSHKeyAlgorithm
        let bits: Int

        var passphrase: String? { isEncrypted ? SSHKeyFixtures.passphrase : nil }
        var testDescription: String { name }
    }

    static let all: [Key] = [
        Key(
            name: "Ed25519 · OpenSSH",
            privateKey: """
                -----BEGIN OPENSSH PRIVATE KEY-----
                b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
                QyNTUxOQAAACAh4hpynmd0ruAV6h1R94hb861wHHujGbpuhlRlt8wlHQAAAKDPJiAwzyYg
                MAAAAAtzc2gtZWQyNTUxOQAAACAh4hpynmd0ruAV6h1R94hb861wHHujGbpuhlRlt8wlHQ
                AAAED8yTayNZ0evJR6YRA12MQtWtc6Tj/Vg+4OZH+mqDHKMiHiGnKeZ3Su4BXqHVH3iFvz
                rXAce6MZum6GVGW3zCUdAAAAHGZpeHR1cmUtZWQyNTUxOS1vcGVuc3NoLW5vbmUB
                -----END OPENSSH PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICHiGnKeZ3Su4BXqHVH3iFvzrXAce6MZum6GVGW3zCUd fixture-ed25519-openssh-none",
            fingerprint: "SHA256:xAkBH7hhH0UcvbA7og4c2msFj6z8kXVit2sHkXKmzrQ",
            algorithm: .ed25519,
            bits: 256),
        Key(
            name: "Ed25519 · OpenSSH · passphrase",
            privateKey: """
                -----BEGIN OPENSSH PRIVATE KEY-----
                b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABDqrKdd2V
                2B6utNtaNCtkF9AAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIPubZIEf+pfR+PyW
                i2MdzOIa4QJL0/qFRbXWuq9ldLPYAAAAoNTu2dxa0v/fhLY8LPbONHvxppeLpeyz5C1EAH
                oqzt97tdhH4k11CjbnBbB8sWH23URbrZjIu8/+0XvemFv/U3rc/oyJ4R5j1LY+QOZbb1PY
                cB8kMCufDPbSh4U2dtuDyRCvRx7LGDaD+t9VZ4ZXQZXz55zIMSkBWGqwyVnrIS49JCMDUH
                gH3lIa0bQSMiueE1tGo4nuKsfg8EryI/dX0T0=
                -----END OPENSSH PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPubZIEf+pfR+PyWi2MdzOIa4QJL0/qFRbXWuq9ldLPY fixture-ed25519-openssh-pass",
            fingerprint: "SHA256:j+R2+kwYLnU/bEsM47JoOgEJehBaEyEaiYT/VFnIJ0w",
            algorithm: .ed25519,
            bits: 256),
        Key(
            name: "ECDSA P-256 · OpenSSH",
            privateKey: """
                -----BEGIN OPENSSH PRIVATE KEY-----
                b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAaAAAABNlY2RzYS
                1zaGEyLW5pc3RwMjU2AAAACG5pc3RwMjU2AAAAQQReS5wqK9tVtqtqBaZOFHCQ0BfPv9xZ
                JNLO51PzXnU39yzFzL1jU9dTTJe49LAMr0QvRyad/Ntu3r672LvFVFYAAAAAuM5cV37OXF
                d+AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBF5LnCor21W2q2oF
                pk4UcJDQF8+/3Fkk0s7nU/NedTf3LMXMvWNT11NMl7j0sAyvRC9HJp38227evrvYu8VUVg
                AAAAAhAJZfFciAIK/zVpcHM0zZWGKbuVy1nSR55OeRW9nPWhDpAAAAGmZpeHR1cmUtZWNk
                c2Etb3BlbnNzaC1ub25lAQIDBAU=
                -----END OPENSSH PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBF5LnCor21W2q2oFpk4UcJDQF8+/3Fkk0s7nU/NedTf3LMXMvWNT11NMl7j0sAyvRC9HJp38227evrvYu8VUVgA= fixture-ecdsa-openssh-none",
            fingerprint: "SHA256:Y82atu0ehMoZKkgZksBoszn80yjGIvGqADbZjBF3FYg",
            algorithm: .ecdsa,
            bits: 256),
        Key(
            name: "ECDSA P-256 · OpenSSH · passphrase",
            privateKey: """
                -----BEGIN OPENSSH PRIVATE KEY-----
                b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABDm0nEHj5
                4Rw5p9RyA0PezgAAAAGAAAAAEAAABoAAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlz
                dHAyNTYAAABBBGygHIFlNSgGgaMbUigTnALuBGL5KI3FsJGvCeCZpNkk1h0N6qsm1qz4YW
                bJzxIE60ZZberNOcJh38GAPEKhOl4AAADAtyGzRYqzFeNZEOU1f0B/ZLhxhknbStfrZcJK
                zqgjOKRW6GmQD7yyP6ij1A521QimmPvahN7878wJGi5FcnSedULWECzRcZudaI/pYlSxfd
                IQlhMCVijNqUz9yoWSJOtlBlL3roielHsxC/85a4eFutr0hpzpf8Ph8Ds7JaPsjSsFfE1+
                /SFCLOTY43mpZK/Vq+gfsUL2W9yp7vMhw7ZGjX61jtgR/DPdW5/cxwNmDDPQilDSzDhXKe
                L3kycZ5KW/
                -----END OPENSSH PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBGygHIFlNSgGgaMbUigTnALuBGL5KI3FsJGvCeCZpNkk1h0N6qsm1qz4YWbJzxIE60ZZberNOcJh38GAPEKhOl4= fixture-ecdsa-openssh-pass",
            fingerprint: "SHA256:APIgVrQOaxMJ/9pRXeVdH3RjwnphRkRLBMwLFIYWzE0",
            algorithm: .ecdsa,
            bits: 256),
        Key(
            name: "ECDSA P-256 · PEM (ssh-keygen -m PEM)",
            privateKey: """
                -----BEGIN EC PRIVATE KEY-----
                MIIBaAIBAQQgTdT3ofyQ6jSiIBqsSWwlQLDm++wP5XghJskOt3t2IaaggfowgfcC
                AQEwLAYHKoZIzj0BAQIhAP////8AAAABAAAAAAAAAAAAAAAA////////////////
                MFsEIP////8AAAABAAAAAAAAAAAAAAAA///////////////8BCBaxjXYqjqT57Pr
                vVV2mIa8ZR0GsMxTsPY7zjw+J9JgSwMVAMSdNgiG5wSTamZ44ROdJreBn36QBEEE
                axfR8uEsQkf4vOblY6RA8ncDfYEt6zOg9KE5RdiYwpZP40Li/hp/m47n60p8D54W
                K84zV2sxXs7LtkBoN79R9QIhAP////8AAAAA//////////+85vqtpxeehPO5ysL8
                YyVRAgEBoUQDQgAEnXowaoEttUDi/q9Q4rpYmhsV9hNE69iAlJrRCA9vEt4oGdsP
                bLaFQWG6Hi7RvuZ+4eLg74/Z1X6QZTeTZ7VIdQ==
                -----END EC PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBJ16MGqBLbVA4v6vUOK6WJobFfYTROvYgJSa0QgPbxLeKBnbD2y2hUFhuh4u0b7mfuHi4O+P2dV+kGU3k2e1SHU= fixture-ecdsa-PEM-none",
            fingerprint: "SHA256:XFbI/FPigXEko28jplfLrxvOZGg2EyWEQXaHTBuks48",
            algorithm: .ecdsa,
            bits: 256),
        Key(
            name: "ECDSA P-256 · PEM (ssh-keygen -m PEM) · passphrase",
            privateKey: """
                -----BEGIN EC PRIVATE KEY-----
                Proc-Type: 4,ENCRYPTED
                DEK-Info: AES-128-CBC,F19E739EA2328E7CDA7C0505C0BB6A71
                
                9qfTXcvW4T5SZr/eWhN86pj/319QgEJPEXeJdXrXz6XInsMy4H3OBM+e9eASonzY
                OzluxEP03jdzWGHLQyNsEk2iOEGbiwoe6wve1IKDUN6SnqWkM84MokiBKJ/ksFG0
                0pM9Er2Vxw6pihA+wIpp8/WwftF1q1IfvU9C8RtnmduXAbOtpMD+atIkwAfC5k00
                0fyriR8qLSjaR2Vsh8ZDgYH5tsYGVdUJLTaQx2YGrgsjRi3bO0gLcqNJZvtToZy9
                EfYI2adCLDx+HuMHHMhObvjEID4ZFfjlGKQQGj5JtdND3EdQjtdoyxXxv3h98q/X
                W4Tjb8OCpNt/lRoSG+OtSFa5pZFQDsol0e+w6d2YfFZAXns/CuzFTU5lF5A60zit
                CcHs+YZAk21twY5MyES+gBK8yPMe3jmsFR4y3v0Yg+I5gszbI5Q13el+z2Yh/VEd
                NLp2JodeuDnozKbeF6CGVTbqHAMIgGQcv1Qnm0GSI5Q=
                -----END EC PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBJO+UPqce+D/OcXUBjzO953Hi48l9UrTgcsLg6YIh6W0F7elqXZBrA7Ls7ZCRBAJd9L6CyVWueL3IAlJ/Ai6TME= fixture-ecdsa-PEM-pass",
            fingerprint: "SHA256:OfiT58XTmkfdooiUphe2fnVwNoLKIXg9cJvdvFCN5+A",
            algorithm: .ecdsa,
            bits: 256),
        Key(
            name: "ECDSA P-256 · PKCS #8 (ssh-keygen -m PKCS8)",
            privateKey: """
                -----BEGIN PRIVATE KEY-----
                MIIBeQIBADCCAQMGByqGSM49AgEwgfcCAQEwLAYHKoZIzj0BAQIhAP////8AAAAB
                AAAAAAAAAAAAAAAA////////////////MFsEIP////8AAAABAAAAAAAAAAAAAAAA
                ///////////////8BCBaxjXYqjqT57PrvVV2mIa8ZR0GsMxTsPY7zjw+J9JgSwMV
                AMSdNgiG5wSTamZ44ROdJreBn36QBEEEaxfR8uEsQkf4vOblY6RA8ncDfYEt6zOg
                9KE5RdiYwpZP40Li/hp/m47n60p8D54WK84zV2sxXs7LtkBoN79R9QIhAP////8A
                AAAA//////////+85vqtpxeehPO5ysL8YyVRAgEBBG0wawIBAQQgmJm261/qcLWf
                5JQQUFpY5nOzirboXTZ97e5YBtT48kahRANCAAQdZI+TRHaVBtane9XXKpZyh3lx
                iAd7CjoQAaviU5ouOVZLszMYagw2D/BTrmU/wOQmtxaLnI8JMKv0pYj1Sr5t
                -----END PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBB1kj5NEdpUG1qd71dcqlnKHeXGIB3sKOhABq+JTmi45VkuzMxhqDDYP8FOuZT/A5Ca3Foucjwkwq/SliPVKvm0= fixture-ecdsa-PKCS8-none",
            fingerprint: "SHA256:8xF7LQ2wk7Q0Z46K5kU3wYsNT6mKEVV32VouZMWkD0M",
            algorithm: .ecdsa,
            bits: 256),
        Key(
            name: "ECDSA P-256 · PKCS #8 (ssh-keygen -m PKCS8) · passphrase",
            privateKey: """
                -----BEGIN ENCRYPTED PRIVATE KEY-----
                MIIBzzBJBgkqhkiG9w0BBQ0wPDAbBgkqhkiG9w0BBQwwDgQIhSmbuGra3XACAggA
                MB0GCWCGSAFlAwQBAgQQ06a4sMXwr1djzUI+mRpKagSCAYCdXb7stqSPYdVdU8Jx
                FxqaN53H/V35J7g/T77FTMopFEke+Hc9wR16rx42Mr1uokhjLHLrFh+bIj2htjFP
                8Pb2nWTgh0hCAbAMcRGWvykqjoUto4yIGr8U2WwmEWSBJJzZiXRFqlHhMsRtdxgG
                lxdVoYd34LB/IJ5DdbtwQtBvtkvmjktV3fFD5DIr7XbBfpZGSEbi3DUu91Q93hZT
                g+mJwURTQfgZy7goXjFL+Drp8KyCRD4FBCInBQRGYOKh9wx+wUYfQJfxN+rVjQBk
                0D9D/D9waftjNvYSI7gKhtoP1sCf1ZEPltZkcRqM66oPCP7DbZ+z3a4ZHYkAf0Vf
                veLu1ojU6NIWEwrRrp3MWyy/AsaQi2LwXcGDDsU6TocFThpLF6Fz5HX56HO6LFz1
                zq2eRTB1/Kf27JdqIzzS8/z6F1KXKaXuQcyj6OwDk0aa36xqIqvvyFMz7yjvAzOq
                vEhTdLTkNvrmPGPFx77FXt0bndEI83kc9EGPxK4OhsyRfOk=
                -----END ENCRYPTED PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBKT0sVMJZ4FosoXtP95QEad1ngMnl6BlKo9fRrIdY13tQQP1MMnz/nDWOmDXKjTmtm6JYsRrSDNz9JfQLZsFcrU= fixture-ecdsa-PKCS8-pass",
            fingerprint: "SHA256:X7qawxpm6LDptiKm9MsvLVKYsPsDhE1ZpRtOQA3sNdE",
            algorithm: .ecdsa,
            bits: 256),
        Key(
            name: "RSA 2048 · OpenSSH",
            privateKey: """
                -----BEGIN OPENSSH PRIVATE KEY-----
                b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAABFwAAAAdzc2gtcn
                NhAAAAAwEAAQAAAQEA5GLPkCArD6P1YAj21F8IDHrLSO1Tw2Hp9VnEOivKOnvvi1V/Afx6
                jjLO1y9A1iK1OyeL1bj3S+z8r4XNRnPeOmfMKDmhPwaVJ8nU1mQkzka41KlgJGeTZ5VqUz
                3/dkVV26dqqY11yk4g5j9AjpEH28n7mzqTdztG3MtC2UKZTreAyJ+YghLldNalOHdjiSK2
                YWcMcCHoDHy2Eone7GGtBKCYVtiVjKIuC7mEQ4RS7rKLWWk/0GoccXefOwkM/k3IeYS1Hn
                i9QnovBCO/KNmToYLq8lPMwPq953Mq4x+YZbPafdTFhPFA3+39pivBHH7sdRjpRYYH6jaa
                sBGKVEA7DQAAA9BXDT2RVw09kQAAAAdzc2gtcnNhAAABAQDkYs+QICsPo/VgCPbUXwgMes
                tI7VPDYen1WcQ6K8o6e++LVX8B/HqOMs7XL0DWIrU7J4vVuPdL7Pyvhc1Gc946Z8woOaE/
                BpUnydTWZCTORrjUqWAkZ5NnlWpTPf92RVXbp2qpjXXKTiDmP0COkQfbyfubOpN3O0bcy0
                LZQplOt4DIn5iCEuV01qU4d2OJIrZhZwxwIegMfLYSid7sYa0EoJhW2JWMoi4LuYRDhFLu
                sotZaT/Qahxxd587CQz+Tch5hLUeeL1Cei8EI78o2ZOhguryU8zA+r3ncyrjH5hls9p91M
                WE8UDf7f2mK8Ecfux1GOlFhgfqNpqwEYpUQDsNAAAAAwEAAQAAAQEAjcNn46aUFBlJ7lqi
                HCEA9f4iwRnPrK/ScaFO1GzqEnNrtalUNP9rc2jGS3vhWwogRJ+/hOJTXqW0E5+SU/EIOG
                jFqRJcrKqBWzlFC+8NiZ0MjbS1IOPJ87ojTZhmQhu3t6ir5KMSQolMkp8gi14q6prtuuUZ
                NCFvUw44aHOWhezlQFVcalGsiDfHYxldLW5FFRewHvZb7f8BzZDjNHpFZO9NvGIV4c6+py
                lNoZA8XTFBwDjqB9oxChNpSVbH8E85G1lBJwrdOUndA8BCa3oSgjXZGinmNZ57qtJXLbRD
                obFBxX1a1RqQQObfzp+ZYTYvlzHOQCUO0SeV/LZ02HJiYQAAAIEAztEnz+/3+JCwyZKbr/
                VRuYzv1ae0zHajXSYRWjJrrbu0xgoVJguUZfxPuPcfj7ldUkZlwawVpM9hq+9Zi2djI+kh
                AKA2wTykZl1jWJawqzNmAVeDF4A9VaTcVRlsr101nBKCg4G7gXcVBeBw4OvuA/BhLmxen2
                ykPKJTj05tGAkAAACBAP7+3ME6ieNakX2h4tMzcFBOH3ZnEQ7x/L2GbE3F02QLfdu0C5yY
                T6M0tTDzkLTVG3q9GFLAQ2oyrnj/ZiBZv2VXCaercnISsa1oTb4m5UN8EO9Vj2g75ENQpu
                Gr6cayXdZsQQNY6u4Xr7agZbyMP1s3SBHlsV7paWnRaxsTWcsLAAAAgQDlSR2H/UoitrZu
                dGRVFlecxzrd1ayLWEDw94hxWYUcUqsaM9fiBfbP+cEvwFVMW3SJEylxKUvyASLxMM/fqO
                QdSoothERxHrjpB9df7uVfoBTTSWDcI/6occHaJ/wQ5TI/74zPGE5h6fq2DBH8tGXsrtRh
                qE4yjfoAqc+oGVWhRwAAABhmaXh0dXJlLXJzYS1vcGVuc3NoLW5vbmUB
                -----END OPENSSH PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDkYs+QICsPo/VgCPbUXwgMestI7VPDYen1WcQ6K8o6e++LVX8B/HqOMs7XL0DWIrU7J4vVuPdL7Pyvhc1Gc946Z8woOaE/BpUnydTWZCTORrjUqWAkZ5NnlWpTPf92RVXbp2qpjXXKTiDmP0COkQfbyfubOpN3O0bcy0LZQplOt4DIn5iCEuV01qU4d2OJIrZhZwxwIegMfLYSid7sYa0EoJhW2JWMoi4LuYRDhFLusotZaT/Qahxxd587CQz+Tch5hLUeeL1Cei8EI78o2ZOhguryU8zA+r3ncyrjH5hls9p91MWE8UDf7f2mK8Ecfux1GOlFhgfqNpqwEYpUQDsN fixture-rsa-openssh-none",
            fingerprint: "SHA256:LiItbQtl6CHkW7yQ+csOZyCHC40L55z+WMHP/04mdGM",
            algorithm: .rsa4096,
            bits: 2048),
        Key(
            name: "RSA 2048 · OpenSSH · passphrase",
            privateKey: """
                -----BEGIN OPENSSH PRIVATE KEY-----
                b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABC6G8YxAe
                u2Wm2Nfn4H+Y+NAAAAGAAAAAEAAAEXAAAAB3NzaC1yc2EAAAADAQABAAABAQDQyyDWbVNq
                2oq32qb4UwMuS1pHcE9ChcMZo/RyVgycgKf8dGuwC/JcqUglxWveq+j3W2bbXNWsoKeNaL
                SGdNMyWJx4/CeKlqFVIKQouajmuSeW3g8HutxfJc1htAjuU1CqFhJt6mGVt6Awq+b72uOK
                2Vx5UuF/e6AAvM0xHKvttkUYyrBNVn+r7UQ6xjc4QooqMRyFQejegG1YDpqQNANx14SuGw
                4KO3pPyh7l+sOF/GCfdc3f0ohYql3CFKOm8u7xY7+rLK+Y6ZNr3wBE9A1jrKMTEa+Kx1rd
                L8h5j1LgXEuEg2WUJuKTsGLoR2CKzNz5ZhQ3C+OZRVSYxLV/JgMhAAAD0L0MuZwb/ZwJyG
                E9vitJ+DaGERjlLZ+qHbHVB6u28ZUlkEWWlxl/VJYqpBxVln2esP6rcjbN2HYlXF7mWdZS
                zDINQL1DoruhsIeOBEdoS2JJdVvsfLgv6M7JGCjic/4jIVPSMXxOVyINVc0nYFWGQeGfmW
                aSAVpaB+5gp3pJi8RPlYWAle1MUKExuFRxSY96PCV1Y9wB58qPLCbA7HyI7sMIXFYPhIOs
                yUAS1R+xzh6s5YDoF85NlR529SFSBLQNQkO7BzY8OD1a/KVrBVUasn5B9MTcVLZWugNc+J
                zbR4rV+DCnxB9yVFjhfTFnf53xUkhdxKceAyKgqaZY1OLvKdNUaVhtkFyE9ofp+AixdVmz
                pm4cJedOwsuqZq+ip0TNJTfa2rViXIQ4KTGRFTkc4A7PB3KY3quSnlAC8ZmzKiBQLE9Uec
                M77Z7f/YsCW/bV8mQGSvjxfAWzZvaUOaoyqLLwUPMreMv7L62j162ARoK2JmDr/ZCmjItr
                0woA38+QlTTpTX+O6n8Vbm9topxJiuxSSVmNzMVXzFW4nYl4ZQveNYBw5ia5k2oYcUDLYc
                6EnaawtpOlh7qdigQGGcSHwM6yJsF5f42nGDGtRBOT9qn/Vk2gUya8e+f1OnMEZEf171/o
                Z8dAK+NwR+tiG96rItIhXJcNX0VwNUIiixRG5d6WYDgYTmKsEKj2H78w+oEWUQXzJG8CCf
                oTi0iUbves+lMnBg+xSVJyKOXJspAkbIvts22FzENthk5Bpj2/QEzWTUHaXqjzjRITxkM8
                +ypqALJFCOOt0HXFq4dT8D1lHK7AaMtmirUKFvKUl6iBDUTq98f5lcXiTl+qb9MacqbfC1
                xH11KTmZZvP4WLxaWkGXqhLXHUsaz5o+RSsdwUj6fZRWNTM02aYJUXQVfqZBJdBdbhS6WH
                O80HhMeTWAlO8xRqIlCY3Hv5WOUQ/jLhyY0QqRKwdE6e1sk12yZAgLjCmAfZwjjOzFTj3b
                E8EOttlA1q5qJLztWJnUFnYsHCk3Ke1mTLOQmyU0BW2duOqwpdvED7Ol+U/moKk1nLN0Ew
                RZGlpZxhEEXt3RUuWsvH/wXinltcHZRG8H3HNS6ObK4IsOztdXI8WYA+0aR2xysPQcIV2i
                Hou1Kw4xQxZdqYoe6t4tsXPPZisoR1VUszZEhybIwfmkGP/AUKfqVccGOyX5KbT/DkKkl3
                E0SGRzVk14TXMItKnrSSfYU2AQ6i7+7+FY6C49BLCkeP0ZnN0HYRY5d9JPwHc5FDpfZWIV
                2NO/TV1WjT6uqlIm8vnLwdxLRnXgI=
                -----END OPENSSH PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDQyyDWbVNq2oq32qb4UwMuS1pHcE9ChcMZo/RyVgycgKf8dGuwC/JcqUglxWveq+j3W2bbXNWsoKeNaLSGdNMyWJx4/CeKlqFVIKQouajmuSeW3g8HutxfJc1htAjuU1CqFhJt6mGVt6Awq+b72uOK2Vx5UuF/e6AAvM0xHKvttkUYyrBNVn+r7UQ6xjc4QooqMRyFQejegG1YDpqQNANx14SuGw4KO3pPyh7l+sOF/GCfdc3f0ohYql3CFKOm8u7xY7+rLK+Y6ZNr3wBE9A1jrKMTEa+Kx1rdL8h5j1LgXEuEg2WUJuKTsGLoR2CKzNz5ZhQ3C+OZRVSYxLV/JgMh fixture-rsa-openssh-pass",
            fingerprint: "SHA256:z2boqPbNa5FJZBqquyGF/Xf5AZsonjyntcR1n7wmIKA",
            algorithm: .rsa4096,
            bits: 2048),
        Key(
            name: "RSA 2048 · PEM (ssh-keygen -m PEM)",
            privateKey: """
                -----BEGIN RSA PRIVATE KEY-----
                MIIEpgIBAAKCAQEA0Hlohj4fFJli6saDiHxVebCFWsCiKE1dUOYVRyXr0S7nmMYi
                uyEfo6/AxS/LlzY0rXjtihsKUCSTWqTRg2k54YSqxjbPZORd5c2JWhfB2ycG0c8e
                Yk9zuoNgayNFa23y5CiFpIJnMJwfhpWXV2aIlhHUVWrZUXsZ1ftmQuyDLBUQtQ7F
                x7YU274mjCjEYdP5Wrjv+rfwxF+IBYDevdk0VUByLQYbc+oXFCOW2lwlsh4GYds8
                zCDR23Co3x62PApk/r0l+2waw9/bXPXHg7BzItw7Dh6k7jslhJTy+udDBqqIkK72
                wyTy/Vp6zTv0Eem2zxCdgWx3de/dD5Izu9Gz/QIDAQABAoIBAQCFYUf4q4IjIcfh
                wcSh7Dqbss5/M8QL2QOdILf4X1Cy8zmFyy6pXxXpGidRgbTWN/4YBsijlY1RnTUR
                x2WsmARKPyEr1CF4wYs53fjExXJoDl4UmlksOP0oFan4ogZmJ2BBjZODn1L6DMk2
                yDqd1qIIsbuSMtx7fzEWcNQKeVurNV3qdGaV+bvu1Q7Lg4u1+Rzmjdn3ucAjNKaU
                Bc+EBMyhvpFGHlslWZr0PhvGGzeooJnZed/GtlbUYVDIlBO0oNrjBMfs0+v2PkR9
                fpS5F4DwFqtXN3NXa0dL5kszKNPrhWuwgINqqv4YDuHx3ssS6OAQF+ZSp0ffdHTl
                cRpYwAuZAoGBAP+yfzrXChsll74T7qf1f/LONT2QumijYIURwcjeZVOGxkNWPwdV
                OVqqcDX2BPv4g+NBhB3ELt1X5NmAPOkop0Bq6x53yLLPOhQOtG1PIcOEDC6x/weF
                S0WC2I1N4XrKP+UA0CRZSuybvdFVEGpsmmfEsTdxZsuryWkp+DxaaUfLAoGBANC4
                mQlR5HaZtHib3kpTm8Vk2V1qwuiNTSZiQ6YlE61iKPAoF4Drjx4jF1kypHiev7gf
                VBj1a6zq2U4YYjh6VVXccLQLGy1hGk1PCt3HamUvho4G/zp10+6qL9HE+uVOz2IQ
                RzmZl+9uRMS4KZpEOMk6dYjMjCuMCz6uxlqeaSpXAoGBALK4IWOQEDk/QluR9Pqi
                hobAiMf6U8B1iS4ZZjgu8eW6yvMf4jgO4mKzxHCC/6LaZdy+20gu6o69M3CuyGQz
                jQtib+pA0R/h5fdE++l3KONOdXFh6h5olV8FgLTrXZGEsbovokv1uX+9r6QT78Bj
                Hl0vRqq48Z+j98gwlJginXmXAoGBAIn0iUpMs3nkjBFvzJRKANqOltEVALDevFCT
                gG0vcnsWN0LM7QQMLjNYNFWDkIwdOVEEe/xnMn0/WNQyb7EuwnqW0nNeJ99toAnb
                Ja6C4QLyT7hOTaIhQSwtlp/K15564b3KTpl2uKLLum5f3zeUwgfXONprgHZEASa0
                NGyu6YnPAoGBAIht9EDAxz30tsE8bo29yGAsBwBQz9sDpiWeBhVI6iSUjTW4pkZu
                +VA9Xe+YWpU4b7sFY6BJEbOu9lEbQ1GEJ6FBZ1gcv9rWuI4abRDikHPgICSHWhBO
                YTID+1mAbgcvgTV+dcuJ3O6l2DVBrKgKBCTD9FwS9t62Sx4LYqiWvr5a
                -----END RSA PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDQeWiGPh8UmWLqxoOIfFV5sIVawKIoTV1Q5hVHJevRLueYxiK7IR+jr8DFL8uXNjSteO2KGwpQJJNapNGDaTnhhKrGNs9k5F3lzYlaF8HbJwbRzx5iT3O6g2BrI0VrbfLkKIWkgmcwnB+GlZdXZoiWEdRVatlRexnV+2ZC7IMsFRC1DsXHthTbviaMKMRh0/lauO/6t/DEX4gFgN692TRVQHItBhtz6hcUI5baXCWyHgZh2zzMINHbcKjfHrY8CmT+vSX7bBrD39tc9ceDsHMi3DsOHqTuOyWElPL650MGqoiQrvbDJPL9WnrNO/QR6bbPEJ2BbHd1790PkjO70bP9 fixture-rsa-PEM-none",
            fingerprint: "SHA256:CR/ABltpTCVdiTo1WNUSd6D1D9Ig94Kj10zjuWz0neA",
            algorithm: .rsa4096,
            bits: 2048),
        Key(
            name: "RSA 2048 · PEM (ssh-keygen -m PEM) · passphrase",
            privateKey: """
                -----BEGIN RSA PRIVATE KEY-----
                Proc-Type: 4,ENCRYPTED
                DEK-Info: AES-128-CBC,598B9CD8DB90BBC6C53BF9B392F66AD6
                
                1W4oHy3EA256aTrt0yG+8e52zwMb7N1JJ0vEpd1sRhklDN+1iWSI5Dzuwdpc4grl
                Ja72IlqpHiY81FSZ2wPTXsMIjH3nh9m0ciLP+FFcGM50LQDgvpPbuVxNmmhBlx8t
                8/sMy4cLCDcepr/0FkUOiYe2sRVNBKYERp5GBEMQ81wYISe5R+5RNepNhBwrTgqI
                p2finqUNVCgJD07oerCOGzya+CbxLFa+X0hQDQDEwd4gnm6Y5j+xg/3tEKY5T4US
                t7//1hzze7Vkh2ihWtvAEFtEOEHqisDojgvBX62bXEtrmUjVpZ7/z2+ME9G7zsjG
                do2KjWmw3t6EY/Q0saeOezPRLPZpq9IihN+We4O7WBzUnNkBzghQUL2XXFrSdQbX
                doD1frtZxq9tNncvAAhs5hoz2Jt/KolqEG06zNKqPaPdYHg7nqU1i11lEZb5YOqp
                ewUaYKceNhTAWHAF2IwX8H4N+XsL8AxnsroeuCAf7t2MsNWQWXoGgVta2jm4zZq1
                d9vnanTCVMUw4ND8CiNDkLRQM/b+ZoMRCk45syIOqO7XU4O1hC6FzOFbAwWAee+h
                nHsYIpHj9ULZ5wwl5VPaxQVj+dQ/DHI4RMD4N7EKktMPm/55tzvxkbK5zgb2yg4d
                xC4XK5lPHN20+4QCtFNdc2bbGJPIWA8qRokSScQ3uWi9/2qANDU4QxBsmqm3uIj0
                WOqsLV6DS+6Khzt4CRnxgeFZfEoCvvKSwTFQ9Q6/DlxJuSPOorrauukD1IbOLiB2
                PN+fubkcHthJ4iYUck+QJ6yd7cCKDM/Ar/zLx0iMJ+5rfOD42tYHobsTQ++wkcMY
                HDgDhvxTpW2LZ/Jq92MT6NeX/rsEtNpTpbUV4ZcmMiSyBpFimSvAALmuRfv+Og5f
                Bnw+xI7fM9yukuDGD36B2pO4Kxtyf4YAwjU69/907lAWCoBGPtDCm4u+VV8TBibD
                CiL8+6PTIre17tYX5dvLfTsjmEuA6a4A3S1XrnE0NgKJKljBcBhsqFuUw0f7+Qgx
                ZyloE78lj1Ma+NZ2nhpdZGVyHwvo71dg1bwV9jH0l7uKPVyaIqXUy/etw7Am3KHM
                STH2wuK/PKhIb7v5SWI8hX5Dup4yldD/v7NuAMk3g3bP634A26wzDaSV7tz/jLFV
                X57cZg2fLjdK33XjR7mPQzksnCTpCuLUZX7rD8sqORMMnXSj1ex1NiRpBekjnA7O
                h/CFV4dDwkPwcweXh2kr5k46x/OX3gH/JW3o93p6CHUPW/ZiywnY73IG1hK26Cco
                CQ7tUjMvvCIa/ZZuEYMASkGmVtokci4/Fnr0dIEtAnV4PfGTGA8pj/wH0Uk7rWh9
                dPZvVv5yGI88jS46aTJ3E72Vin172bgqPF4gVZGnjSxdEsfFR5k0MIu1FWi2ico+
                yXIL79p2gtjFYjKOvOyv7LxDilTxKQG9xTTTnFGSyBz9IsTH7WymSAIROptoc3Ro
                FejU1xPmxg2KpWzvuNvmTz2+0LIiNt5IIF3050/DFTN65qeY8erKCuQPJcgV+kLj
                TIq+UiWaTAjTa6U76Y24DTdFaNhFqy+BhyGSl+9EWt7HJSolM4PGOiBjH6kKNSeb
                -----END RSA PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQCsHV33CiaAnRA3YbZjswpw4ZOiUjdiFT/D2QiGQoII+Q9F6M6rWm7hbrFcjiqhcRQ9reaasQDlJthaqJjoijCxgG+P/GClPrqCO4xfNw8E30HX9GL13PGV6OZMgMq6PjdzmoNeB5mzRmHxQrHsbmDMwKPdoaguFmcGB3Ant4MGsMFDMvWM223ZXEsTDULSPB21SPTOUBAhhuwt2inFAJHX/Hk9FbkX5n+Rwr+6GsWM4iUGODKQRTpo12gvmhz6vdGWuKt/HY8kRfoMviAuIgt1k0ee2BNR+6HhegnJdamhbXYfqxmZa2G3kI4dNe76Aafm6T5YMxN8np0eHCeNajJ5 fixture-rsa-PEM-pass",
            fingerprint: "SHA256:BsGnb2OvohFu0VeAdA9X1MUFgbrPaY9qjHtj9OLTw9I",
            algorithm: .rsa4096,
            bits: 2048),
        Key(
            name: "RSA 2048 · PKCS #8 (ssh-keygen -m PKCS8)",
            privateKey: """
                -----BEGIN PRIVATE KEY-----
                MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDFdOSrLWixba+m
                BIht0Mid/oHxZoNQ336Qhmbpbv1PbuLPfMnD+bEBf3mf60hhvA0ZOJpJA78ZRQ4X
                uguHSJ1mUxx8ux5kSYm0GpryngTHbXrMXn+246OVyV0jTBaqKcQQbCqO4+5lSfhj
                JyFxkZocCOXBaLuXIkJs/6OKCP0aYbLJCnKsMmIKMRW95rDZuMOy9xh6IjB7WbYU
                YsUqnZMZXQF09bB/VOCsPgmEA7RREp+Wku7iwUqo2j0r4SDWdIAMMpqGO+AkJbGA
                Psuv5OnG6Ecc6+zkiZbWYDvoroK0ewt6ziICcW24PI6NfY4QUPl4XC172XChKs4F
                LWdMnxjRAgMBAAECggEBAK937ElN5g5eOcwflRl5gv4cEF90SNeSdId8WHGyxvpS
                wM2/yBEWj7yUCoIZdM4dbcpu6XRfFvAe1KIzG1GSLwWib9tL8XsPeoaJag6WbArI
                +Goqw/XDnzhZpCHc2PtIeEpbxQoL9Wsfa9K5EFg7Gqc1JO9V4fYxZLQ2DJ9602kf
                vwFYusUCRfbyTPRxHEPRLLCv7jpOdviogoo/8p2wYZXOZPc2bEELEfb9dQNYy03Z
                mG+p3eEx5/5bTWwwE7nMXxqtMHGZB6QC3qmUjfE/huf/Teg4kY8I7G2LRC7ikXVC
                v0DClOC7AOL5fG3XWJAfKd9kxD1DZXl/j4NQt7vNPgECgYEA6Vc/C0pn2t4/Ni8y
                J7FbZG471WdGvFcpCJE5wpKvc5h6dKH1bT/hHAklxVuFDIZErq+BS5T4hFGJqlSI
                vgyzSGu0tnB5gldb0zaLarjIbCdANq8yEx2g1A/qwoCLjUg8JbuPby7xCTVbCwkK
                HxfSR3VRM5ygIAlhATegWJJDPPECgYEA2KGUxhjL2MooSI7k2Zgzu+mVl2Y7QHy4
                rwMn5guN3GFvGBuCrRJhw2xyIsqbt4EBfgd31ueKrRmUnGQOvJVm2xEe+Fmav0Gi
                /E3TySuN2Ihl5XwjJ5GvKGa7KiAprHxvH+UwbEdOZdj+uuSJfT3MY5crinac2gva
                BkAolvPDGeECgYAXfP3OoCXGJVsNlECjODov6NzDBYtcSBXMU/oRePyKUU36UZdH
                8Qcy8ETKEgVZlmZxgx8rwYomn6KQ78N4BgVCA/2D49yHX03R/AqaqpEaOfU/y/Dp
                dbELx7Jk89WYWi4byj6M/ZeryqAogzdum9JB+TA8VsbTkr4eBkBwygBUsQKBgQCg
                J1z40y4pONNtSQ8Rez+MBhWM14yokUxsPrYJkbMwIWxjczs5vqQ+OPdsl0Nl997l
                kJndbFKOxKGkuShX7VAcVt0BmCUajmjzhIo1aJi2Y6iJbywCPeczxDdM3nhQx7HT
                77nL4y+nhjRUcU7gI4bufb034Zb2cNqf0mDvsIkloQKBgHDhJR4tke7RMm0D1bwe
                1b41p3wpqf2UXLO3GdvL9aJbo8jDRyHz64OA767q4d85E8WGI7Rov4esV8LCZxX6
                C9LzuG7WD0Abi0OxO2Ym60lE2CM8AoozZnfL+kxqlk+aEkzO+Tg6iCC0TzEOWdPt
                Ku2ryZBgQ3wvtTeWqRLPmXkX
                -----END PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDFdOSrLWixba+mBIht0Mid/oHxZoNQ336Qhmbpbv1PbuLPfMnD+bEBf3mf60hhvA0ZOJpJA78ZRQ4XuguHSJ1mUxx8ux5kSYm0GpryngTHbXrMXn+246OVyV0jTBaqKcQQbCqO4+5lSfhjJyFxkZocCOXBaLuXIkJs/6OKCP0aYbLJCnKsMmIKMRW95rDZuMOy9xh6IjB7WbYUYsUqnZMZXQF09bB/VOCsPgmEA7RREp+Wku7iwUqo2j0r4SDWdIAMMpqGO+AkJbGAPsuv5OnG6Ecc6+zkiZbWYDvoroK0ewt6ziICcW24PI6NfY4QUPl4XC172XChKs4FLWdMnxjR fixture-rsa-PKCS8-none",
            fingerprint: "SHA256:q3iPPU651gJ9F4FRZFp1mmQYawsEQQKDh7DAh9+gLiY",
            algorithm: .rsa4096,
            bits: 2048),
        Key(
            name: "RSA 2048 · PKCS #8 (ssh-keygen -m PKCS8) · passphrase",
            privateKey: """
                -----BEGIN ENCRYPTED PRIVATE KEY-----
                MIIFHzBJBgkqhkiG9w0BBQ0wPDAbBgkqhkiG9w0BBQwwDgQI2AHMthmeKmkCAggA
                MB0GCWCGSAFlAwQBAgQQ0PToQLzU0EA+4UMI7CiB/ASCBNA0yZ7W1IllOqYRDXoj
                3L0Uh/ck4DQyUKZH+AVklYi86fW0woEk936bRN/+4RgvPjk2A1Do9AkVKvIYaIIq
                yCGUwQ3A1dEvfrhm49OTXbEyQnF+Ggdc7qzJKFNCOxy70PmY0qE8wbkA51hvz1a2
                MCfhmBgVKkHex8KQBmkhRtstiGkkeDrcNqgdqR9OKbgMabLeHEYV8ALw2hulVnMC
                5ukaENLBhGlFohqhCOkypIlUzCzw8qzoUhCfSJcs/r1Ez068Q/9uYO6HKPSJqbeG
                uRizk6famcWJFW9HMDqM0zCa/erwgFq3eH7OCx0wB5HnN0nHU+RHCQVNddh1JP/w
                4HY5q1yzaqB5B8PqtueVG3S99CbYPZY9oGa4yH73pHiHkqBJu6V7fopQTH6nL6zX
                ot7RMiKQWUMcR3nluHXjB2OG61yHZeT1RwekG31np5gURtivCaaYAbkDaKSrfg4U
                RwpCmP7hAPW4OxMLF+ZT0UTcGtqqb4edHUdi2SijId2K2KbbPqHzEJsOi1tsXWGq
                nZAddTaNlN8vcF5TcYbanKQTgxGJFAyuLMACnwPqYoDuPMTDcMjHlkR6/inwvo4K
                4WZ/8RGfZOfCFWSTSf8ZCC31NdPk3XkdlxKnCHdajynLplcxzIryNgcfd14WBJMY
                8ARq3hfE2gzZpXT8KaibYpaDuwNC/k1xzKlmWJZauvIPbRy7SuxEM5PP+hxMtfqa
                ViTyxdsYAIavwS15GHYtBl21a5o0p4J/RilZVuaYF4xsZdJPr8cTWXM/o3LzOlX/
                qqbxSlH3n4ELIbXNsuJ3g/uEXskKEE+x4DLPMEu82GXYlwIdkfRFYN7RxN9eKpas
                xCyLC6RmprgPYcy5TnpIGLUnSJervaaXoE9UG27vRGRSLs0BFpzCUvtZrORbhoOK
                ZyekiYwPC2C1m//bYvJn3gnopH2GAe/ClKMOPjf34Fgsp+fOmumUx1IWk+vQLrcR
                gz+3T0Pn7pBXGbARB6HicuS5EEEJUd9ZN9VQUBNpCiHk2w/d+PpjEggf5mhb8h6W
                D7AIVfVX7lr+wgWKbEU2bXgBQGIClm/XCkWYsT7pz37szUQIvEKhsB/mmqz370es
                5L9urRyuDgDLuhnlvKL+W42eyUfdYsyjpv0qTGK0CvDJog7tJM6YpUO8pewG0atu
                djfEM6MclAfw2Oll/lUl+91HOse5a3DR07cgdG3ydO5CaX5r9I7LNTs4xXrU5N9Y
                s/1sTe8OOmH6j/0mcyqhasWp4Od0cxO7NsWhrEShmHskRqLpEbCpgZjP3uhbNg/7
                zy/jNaQQ25yzZlTzsFZo9vEig71xwjuG/hdvnr7TO+8tc31C0n9JDpj8ZC4ARAsY
                J1g6gMCr/dX5KOf7nvtb9rmMuybhfIQlvvjSYCdFxqg8w5+VqSJ0BdhHsQ9l7Tk1
                4RWUi7PzcTjXSyEFWaBmkdXBrb4y4nnjdLt92zfWeft/9uMo1nGWhb7GPcLM6xFV
                Ni3kIpbsuEFMk1OT+GTouTdscYp+vfTEy0EiMaYblrzIc6GAWM6WRFcWqJYam5+l
                V1N66+Uv8vwG/z6N7Vl1Bf2sKLUV3egCDY9iTOn69XMBNpsxyGDUL2aV2js1WCS+
                33XO+rzQpPOLZOQgwBnoAHD2eA==
                -----END ENCRYPTED PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDXuy0LusZMU9tAnYIKjUHj1eBOvtNbfPt5B0YAQvnqUqe67NTGrJE6/8F0zE9yzjvjYioFBsYNjrM1jL/shVy9m4DeqFUe3vxymMhDfzGsrlDGADscw6MVAtFlW028lwQEsYHPuUbC+HOfcRRTxX76VVy9IKMnXyXsSOluBm7hMCSNUvd/cx6iTVwDIAOW93h+aFkrXV+hlBr83bBdzeSRY6HWieAD3T0SXsyNZOtFHevkRkACYLn6Fk0GlLi98+qGMz6F4rZ102i7FD1LHErXiWQovExgMOMKzWSfazla8bR9WDnvXn+aT9T9pDwyc4q7IH2q8KPQrngAEA4o9qxF fixture-rsa-PKCS8-pass",
            fingerprint: "SHA256:3nXnCQ7kew+BTgpc+2MHoaCfsg5D2N3p5mlG5zKlB1s",
            algorithm: .rsa4096,
            bits: 2048),
        Key(
            name: "Ed25519 · PKCS #8 (openssl genpkey)",
            privateKey: """
                -----BEGIN PRIVATE KEY-----
                MC4CAQAwBQYDK2VwBCIEIH1FOBVVwapEqRPhGM8p7m1ys09beGvzArMGpfdFrtBR
                -----END PRIVATE KEY-----
                """,
            isEncrypted: false,
            publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIO/Fd9cU40IzSCX6Y4tYO6U8qSv1cntEIyhWNeZOfMoZ",
            fingerprint: "SHA256:yWRwu5241ntvp30i+4Y9dJ9Cz4E5v5gSbi1flri6KWI",
            algorithm: .ed25519,
            bits: 256),
        Key(
            name: "Ed25519 · PKCS #8 (openssl genpkey) · passphrase",
            privateKey: """
                -----BEGIN ENCRYPTED PRIVATE KEY-----
                MIGjMF8GCSqGSIb3DQEFDTBSMDEGCSqGSIb3DQEFDDAkBBAYLxlO+2tUvvJZPunQ
                Qt3TAgIIADAMBggqhkiG9w0CCQUAMB0GCWCGSAFlAwQBKgQQUGGByP6p3MiJfDsl
                z2WdVARAYdWeJcUkwVrFXWL3pzCcTxu32+nnCRXEO+oVE9wsbbTrRuj0LJJUzMW+
                7shcFg3V/GeMBNClzGpXpmJ1NZqUvQ==
                -----END ENCRYPTED PRIVATE KEY-----
                """,
            isEncrypted: true,
            publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIO/Fd9cU40IzSCX6Y4tYO6U8qSv1cntEIyhWNeZOfMoZ",
            fingerprint: "SHA256:yWRwu5241ntvp30i+4Y9dJ9Cz4E5v5gSbi1flri6KWI",
            algorithm: .ed25519,
            bits: 256),
    ]

    static var encrypted: [Key] { all.filter(\.isEncrypted) }

    // MARK: Keys the connect path can't sign with

    /// `ssh-keygen -t ecdsa -b 384`: a NIST curve `SSHService` doesn't load.
    static let ecdsaP384 = """
        -----BEGIN OPENSSH PRIVATE KEY-----
        b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAiAAAABNlY2RzYS
        1zaGEyLW5pc3RwMzg0AAAACG5pc3RwMzg0AAAAYQQc2p3czxyToz+vBpbg2U57YeWXVik1
        9a0C8foFzL08pYAIpAH0OnzqxT4cqskjBp10FZz8sDftGq4cNRQlEEiUAQp4fYcsz/s3yK
        dYysL9OZQIeE/VtTD/Nv1/xOAwnEwAAADg++iqHvvoqh4AAAATZWNkc2Etc2hhMi1uaXN0
        cDM4NAAAAAhuaXN0cDM4NAAAAGEEHNqd3M8ck6M/rwaW4NlOe2Hll1YpNfWtAvH6Bcy9PK
        WACKQB9Dp86sU+HKrJIwaddBWc/LA37RquHDUUJRBIlAEKeH2HLM/7N8inWMrC/TmUCHhP
        1bUw/zb9f8TgMJxMAAAAMH47Ndbgaq5rV50Go1npzOfH9oGUlmXRfsNvlbI2cyN/68kofz
        vJWbKIHzbZIe2FkAAAABJjbHVhc0BtYWMtbWluaS5sYW4BAgMEBQY=
        -----END OPENSSH PRIVATE KEY-----
        """

    /// `ssh-keygen -t ed25519 -Z chacha20-poly1305@openssh.com`, with
    /// `passphrase`: the one cipher `ssh-keygen -Z` offers that has no AES.
    static let chachaEncrypted = """
        -----BEGIN OPENSSH PRIVATE KEY-----
        b3BlbnNzaC1rZXktdjEAAAAAHWNoYWNoYTIwLXBvbHkxMzA1QG9wZW5zc2guY29tAAAABm
        JjcnlwdAAAABgAAAAQ0eGtpFDFeeQh/tnmRVoJDAAAAAQAAAABAAAAMwAAAAtzc2gtZWQy
        NTUxOQAAACAkuQ8D9MGo8KSHxMJl9q38KuNivfJ3FEgNVQF3UuxZiAAAAKjVwxhUmN8QxS
        RsQMu5cz5YPq92WmVqJAVhNlPLHSV0IYDlIJ5keCJuI7TZ9MNGCoAs2X9qNOTwupdY4x/U
        /F8uLXUZRapkOM1upySBPomRJ/hhom7vdlyrk5mBAEYkdHJ2GXj7EiXGY/i68VA9fvudLa
        6lJmLbTDP/VMjRwkN0SOvPKmpEAETILUeY2VqH6rI5nJe7Ravnb5OsD4U8hS4MvTuyytjD
        Jt3sTxjVX7KwMBtP8jQMZqdZ
        -----END OPENSSH PRIVATE KEY-----
        """

    /// `openssl pkcs8 -topk8 -v1 PBE-SHA1-3DES`, with `passphrase`: a PKCS #12
    /// password scheme from before PBES2.
    static let pkcs12Encrypted = """
        -----BEGIN ENCRYPTED PRIVATE KEY-----
        MIGxMBwGCiqGSIb3DQEMAQMwDgQIDcDdsWVe3YkCAggABIGQVDsf5rt6NYgJZ3Kb
        shTFvpVMFLqkxKQmfeLWDa1vRGkFKhYSMQVKX97sjMJY1ZYm5d+mETkyEOq5LQP2
        m+wSCHhhlWo9pqvSamawrgG6vIOtLHl1p5sDlIgRq/9Rqzxf7vbZSgYEk3wTQ7dy
        Pkp0K6tTeO04cq/6XNnq1r9qCeCw7gW0HM6cjBn7Yh1n4izV
        -----END ENCRYPTED PRIVATE KEY-----
        """
}

extension StoredKey {
    /// Whether 1.0.8's connect path reads `text` as stored: its first step,
    /// Citadel's key detection, takes only an openssh-key-v1 file with
    /// nothing but "\n" between its lines. What the tests hold
    /// ``assess(_:passphrase:)``'s "usable" to (the app no longer asks
    /// Citadel: the Rust reader decides, the same way).
    static func connectPathReads(_ text: String) -> Bool {
        (try? SSHKeyDetection.detectPrivateKeyType(from: text)) != nil
    }
}
