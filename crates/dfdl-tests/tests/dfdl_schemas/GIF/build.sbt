val root = (project in file("."))
  .settings(
    name := "dfdl-gif",

    organization := "com.mitre",

    version := "0.0.1"
  )
  .daffodilProject()
