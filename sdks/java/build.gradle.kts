plugins {
    `java-library`
    `maven-publish`
    signing
}

group = "dev.sutradb"
version = "0.3.0"

java {
    sourceCompatibility = JavaVersion.VERSION_11
    targetCompatibility = JavaVersion.VERSION_11
    withSourcesJar()
    withJavadocJar()
}

repositories {
    mavenCentral()
}

dependencies {
    api("org.json:json:20240303")
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.3")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

tasks.test {
    useJUnitPlatform()
}

tasks.javadoc {
    (options as StandardJavadocDocletOptions).addStringOption("Xdoclint:none", "-quiet")
}

publishing {
    publications {
        create<MavenPublication>("mavenJava") {
            from(components["java"])

            pom {
                name.set("SutraDB Java Client")
                description.set("Java client for SutraDB — RDF-star triplestore with native vector indexing")
                url.set("https://github.com/EmmaLeonhart/SutraDB")

                licenses {
                    license {
                        name.set("Apache License, Version 2.0")
                        url.set("https://www.apache.org/licenses/LICENSE-2.0")
                    }
                }

                developers {
                    developer {
                        name.set("SutraDB Contributors")
                        url.set("https://github.com/EmmaLeonhart/SutraDB")
                    }
                }

                scm {
                    url.set("https://github.com/EmmaLeonhart/SutraDB")
                    connection.set("scm:git:https://github.com/EmmaLeonhart/SutraDB.git")
                    developerConnection.set("scm:git:git@github.com:EmmaLeonhart/SutraDB.git")
                }
            }
        }
    }

    repositories {
        maven {
            name = "central"
            url = uri("https://central.sonatype.com/repository/maven-releases/")
            credentials {
                username = System.getenv("MAVEN_USERNAME") ?: ""
                password = System.getenv("MAVEN_TOKEN") ?: ""
            }
        }
        maven {
            name = "centralSnapshots"
            url = uri("https://central.sonatype.com/repository/maven-snapshots/")
            credentials {
                username = System.getenv("MAVEN_USERNAME") ?: ""
                password = System.getenv("MAVEN_TOKEN") ?: ""
            }
        }
    }
}

signing {
    // Only sign when GPG key is available (CI or manual release)
    val signingKey = System.getenv("GPG_PRIVATE_KEY")
    val signingPassword = System.getenv("GPG_PASSPHRASE")
    if (signingKey != null) {
        useInMemoryPgpKeys(signingKey, signingPassword)
    }
    sign(publishing.publications["mavenJava"])
    isRequired = signingKey != null
}
