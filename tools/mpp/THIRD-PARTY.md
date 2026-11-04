# Third-party software in Super PM's .mpp converter

Super PM reads Microsoft Project `.mpp` files with a small converter (`MppToXml`, part of Super
PM) that runs on the libraries below and a reduced Java runtime. They are shipped as separate,
unmodified files inside the app (`Super PM.app/Contents/Resources/mpp/lib`), so each can be
replaced with another version of the same library.

| Library | Version | License |
|---|---|---|
| MPXJ (net.sf.mpxj:mpxj) | 16.10.0 | GNU LGPL 2.1 or later ([text](licenses/LGPL-2.1.txt)); source: https://github.com/joniles/mpxj |
| Apache POI (org.apache.poi:poi) | 5.5.1 | Apache 2.0 ([text](licenses/Apache-2.0.txt)) |
| Apache Commons Codec | 1.20.0 | Apache 2.0 |
| Apache Commons Collections | 4.5.0 | Apache 2.0 |
| Apache Commons IO | 2.21.0 | Apache 2.0 |
| Apache Commons Math | 3.6.1 | Apache 2.0 |
| Apache Log4j API | 2.25.5 | Apache 2.0 |
| SparseBitSet (com.zaxxer) | 1.3 | Apache 2.0 |
| RTF Parser Kit (com.github.joniles) | 1.16.0 | Apache 2.0 |
| Jakarta XML Binding API | 3.0.1 | Eclipse Distribution License 1.0 ([text](licenses/EDL-1.0.md)) |
| Eclipse Implementation of JAXB (jaxb-runtime, jaxb-core, txw2) | 3.0.2 | Eclipse Distribution License 1.0 |
| istack commons runtime | 4.0.1 | Eclipse Distribution License 1.0 |
| Jakarta Activation | 2.0.1 | Eclipse Distribution License 1.0 |
| OpenJDK runtime (jlink image) | 21 | GNU GPL 2 with the Classpath Exception; its notices are in `runtime/legal` |

Each library's own license and notice files are also inside its `.jar` (`META-INF`).
