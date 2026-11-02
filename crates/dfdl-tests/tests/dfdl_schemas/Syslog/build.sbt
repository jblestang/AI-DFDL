val root = (project in file("."))
  .settings(
    name := "dfdl-syslog",

    organization := "com.owlcyberdefense",

    version := "2.0.1"
  )
  .daffodilProject()
