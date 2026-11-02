val root = (project in file("."))
  .settings(
    name := "dfdl-tcpMessage",
    organization := "io.github.dfdlschemas",
    version := "1.2.0"
  )
  .daffodilProject()
