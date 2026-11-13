plugins {
	kotlin("jvm") version "2.0.21"
	kotlin("plugin.serialization") version "2.0.21"
}

group = "dev.leptoskit"
version = "0.1.0"

repositories {
	mavenCentral()
}

dependencies {
	testImplementation(kotlin("test"))
	testImplementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
}

kotlin {
	jvmToolchain(21)
}

tasks.test {
	useJUnitPlatform()
}
