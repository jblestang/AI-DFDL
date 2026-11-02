val root = (project in file("."))
  .settings(
    name := "dfdl-ntp",
    organization := "com.owlcyberdefense",
    version := "1.0.0",

    daffodilPackageBinInfos := Seq(
      DaffodilBinInfo("/com/owlcyberdefense/ntp/xsd/ntp-root.dfdl.xsd")
    ),
    daffodilTdmlUsesPackageBin := true
  )
  .daffodilProject()

