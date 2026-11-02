val root = (project in file("."))
  .settings(
    name := "dfdl-ethernetIP",

    organization := "com.owlcyberdefense",

    version := "1.5.0",

    daffodilBuildsLayer := true
  )
  .daffodilProject(crossDaffodilVersions = Seq("4.0.0"))
