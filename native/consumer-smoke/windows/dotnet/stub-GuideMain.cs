// Stub GuideMain for testing before extraction
// This is a minimal main that just confirms compilation
// Replace with generated code from extract-guide-blocks.ps1

using MyApp;

namespace NativeOidcGuideApp
{
    public static class Program
    {
        public static async Task<int> Main(string[] args)
        {
            Console.Out.WriteLine("NativeOidcGuideApp: Guide checks runner");
            
            // For now, just exit successfully - actual guide checks are in GuideChecks.cs
            // which will be generated and compiled when the pre-build target runs
            
            if (args.Length > 0 && args[0] == "dotnet-guide")
            {
                Console.Out.WriteLine("NativeOidcGuideApp: dotnet-guide mode not needed - running checks directly");
                return await NativeOidcPackage.GuideChecks.RunAsync("http://127.0.0.1");
            }

            return 0;
        }
    }
}
