using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Win32.SafeHandles;

namespace ProjectFlora.NativeOidc;

/// <summary>Runs Windows browser authorization through the native Rust library.</summary>
public static class NativeOidcWindows
{
    /// <summary>
    /// Runs authorization on a worker thread and returns serialized authentication state.
    /// The returned JSON contains sensitive tokens and must be stored securely.
    /// </summary>
    public static Task<string> AuthorizeAsync(
        string requestJson,
        CancellationToken cancellationToken = default)
    {
        if (requestJson is null)
        {
            throw new ArgumentNullException(nameof(requestJson));
        }

        return Task.Run(() =>
        {
            var cancellationPointer = NativeMethods.CreateCancellation();
            if (cancellationPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native cancellation handle could not be created.");
            }
            using var cancellation = new NativeMethods.CancellationHandle(cancellationPointer);
            using var registration = cancellationToken.Register(
                () => NativeMethods.Cancel(cancellation));
            var resultPointer = NativeMethods.Authorize(requestJson, cancellation);
            if (resultPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native authorization call returned no result.");
            }
            using var result = new NativeMethods.AuthorizationResultHandle(resultPointer);

            var valuePointer = NativeMethods.ResultValue(result);
            var value = Marshal.PtrToStringUTF8(valuePointer)
                ?? throw new InvalidOperationException("The native authorization result is empty.");
            return NativeMethods.ResultStatus(result) switch
            {
                0 => value,
                2 => throw new OperationCanceledException(cancellationToken),
                _ => throw new InvalidOperationException(value),
            };
        }, CancellationToken.None);
    }

    /// <summary>
    /// Refreshes tokens on a worker thread and returns the updated
    /// serialized authentication state. The returned JSON contains sensitive
    /// tokens and must be stored securely.
    /// </summary>
    public static Task<string> RefreshAsync(
        string requestJson,
        CancellationToken cancellationToken = default)
    {
        if (requestJson is null)
        {
            throw new ArgumentNullException(nameof(requestJson));
        }

        return Task.Run(() =>
        {
            var cancellationPointer = NativeMethods.CreateCancellation();
            if (cancellationPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native cancellation handle could not be created.");
            }
            using var cancellation = new NativeMethods.CancellationHandle(cancellationPointer);
            using var registration = cancellationToken.Register(
                () => NativeMethods.Cancel(cancellation));
            var resultPointer = NativeMethods.Refresh(requestJson, cancellation);
            if (resultPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native refresh call returned no result.");
            }
            using var result = new NativeMethods.AuthorizationResultHandle(resultPointer);

            var valuePointer = NativeMethods.ResultValue(result);
            var value = Marshal.PtrToStringUTF8(valuePointer)
                ?? throw new InvalidOperationException("The native refresh result is empty.");
            return NativeMethods.ResultStatus(result) switch
            {
                0 => value,
                2 => throw new OperationCanceledException(cancellationToken),
                _ => throw new InvalidOperationException(value),
            };
        }, CancellationToken.None);
    }

    /// <summary>
    /// Runs OIDC end-session (RP-initiated logout) on a worker thread.
    /// With a post-logout redirect URI, the returned JSON is the validated
    /// logout response; without one, the system browser is opened and the
    /// returned string is empty.
    /// </summary>
    public static Task<string> EndSessionAsync(
        string requestJson,
        CancellationToken cancellationToken = default)
    {
        if (requestJson is null)
        {
            throw new ArgumentNullException(nameof(requestJson));
        }

        return Task.Run(() =>
        {
            var cancellationPointer = NativeMethods.CreateCancellation();
            if (cancellationPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native cancellation handle could not be created.");
            }
            using var cancellation = new NativeMethods.CancellationHandle(cancellationPointer);
            using var registration = cancellationToken.Register(
                () => NativeMethods.Cancel(cancellation));
            var resultPointer = NativeMethods.EndSession(requestJson, cancellation);
            if (resultPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native end-session call returned no result.");
            }
            using var result = new NativeMethods.AuthorizationResultHandle(resultPointer);

            var valuePointer = NativeMethods.ResultValue(result);
            var value = Marshal.PtrToStringUTF8(valuePointer)
                ?? throw new InvalidOperationException("The native end-session result is empty.");
            return NativeMethods.ResultStatus(result) switch
            {
                0 => value,
                2 => throw new OperationCanceledException(cancellationToken),
                _ => throw new InvalidOperationException(value),
            };
        }, CancellationToken.None);
    }

    /// <summary>
    /// Requests OAuth 2.0 token revocation on a worker thread.
    /// This does not delete local authentication state.
    /// </summary>
    public static Task RevokeAsync(
        string requestJson,
        CancellationToken cancellationToken = default)
    {
        if (requestJson is null)
        {
            throw new ArgumentNullException(nameof(requestJson));
        }

        return Task.Run(() =>
        {
            var cancellationPointer = NativeMethods.CreateCancellation();
            if (cancellationPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native cancellation handle could not be created.");
            }
            using var cancellation = new NativeMethods.CancellationHandle(cancellationPointer);
            using var registration = cancellationToken.Register(
                () => NativeMethods.Cancel(cancellation));
            var resultPointer = NativeMethods.Revoke(requestJson, cancellation);
            if (resultPointer == IntPtr.Zero)
            {
                throw new InvalidOperationException("The native revocation call returned no result.");
            }
            using var result = new NativeMethods.AuthorizationResultHandle(resultPointer);

            var valuePointer = NativeMethods.ResultValue(result);
            var value = Marshal.PtrToStringUTF8(valuePointer)
                ?? throw new InvalidOperationException("The native revocation result is empty.");
            switch (NativeMethods.ResultStatus(result))
            {
                case 0:
                    return;
                case 2:
                    throw new OperationCanceledException(cancellationToken);
                default:
                    throw new InvalidOperationException(value);
            }
        }, CancellationToken.None);
    }

    /// <summary>
    /// Returns the validated ID Token claims of serialized authentication
    /// state as a JSON object with <c>iss</c>, <c>sub</c>, <c>aud</c>,
    /// <c>exp</c>, <c>iat</c>, and, when present, <c>auth_time</c>,
    /// <c>acr</c>, <c>amr</c>, <c>azp</c>, and <c>additional_claims</c>, or
    /// <c>null</c> when the state has no readable ID Token. Accepts state
    /// returned by <see cref="AuthorizeAsync"/> or <see cref="RefreshAsync"/>
    /// and state saved by earlier versions. Makes no network request.
    /// Bind accounts to <c>iss</c> and <c>sub</c> together.
    /// </summary>
    public static string? GetIdTokenClaims(string stateJson)
    {
        if (stateJson is null)
        {
            throw new ArgumentNullException(nameof(stateJson));
        }

        var resultPointer = NativeMethods.IdTokenClaims(stateJson);
        if (resultPointer == IntPtr.Zero)
        {
            throw new InvalidOperationException("The native ID Token claims call returned no result.");
        }
        using var result = new NativeMethods.AuthorizationResultHandle(resultPointer);

        var valuePointer = NativeMethods.ResultValue(result);
        var value = Marshal.PtrToStringUTF8(valuePointer)
            ?? throw new InvalidOperationException("The native ID Token claims result is empty.");
        if (NativeMethods.ResultStatus(result) != 0)
        {
            throw new InvalidOperationException(value);
        }
        return value == "null" ? null : value;
    }

    private static class NativeMethods
    {
        private const string Library = "native_oidc.dll";

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_cancellation_new")]
        internal static extern IntPtr CreateCancellation();

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_cancellation_cancel")]
        internal static extern void Cancel(CancellationHandle handle);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_cancellation_free")]
        private static extern void FreeCancellation(IntPtr handle);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_authorize")]
        internal static extern IntPtr Authorize(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string requestJson,
            CancellationHandle cancellation);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_refresh")]
        internal static extern IntPtr Refresh(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string requestJson,
            CancellationHandle cancellation);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_end_session")]
        internal static extern IntPtr EndSession(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string requestJson,
            CancellationHandle cancellation);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_revoke")]
        internal static extern IntPtr Revoke(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string requestJson,
            CancellationHandle cancellation);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_id_token_claims")]
        internal static extern IntPtr IdTokenClaims(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string stateJson);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_authorization_result_status")]
        internal static extern int ResultStatus(AuthorizationResultHandle result);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_authorization_result_value")]
        internal static extern IntPtr ResultValue(AuthorizationResultHandle result);

        [DllImport(Library, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true,
            EntryPoint = "native_oidc_windows_authorization_result_free")]
        private static extern void FreeResult(IntPtr result);

        internal sealed class CancellationHandle : SafeHandleZeroOrMinusOneIsInvalid
        {
            internal CancellationHandle(IntPtr handle) : base(ownsHandle: true)
            {
                SetHandle(handle);
            }

            protected override bool ReleaseHandle()
            {
                FreeCancellation(handle);
                return true;
            }
        }

        internal sealed class AuthorizationResultHandle : SafeHandleZeroOrMinusOneIsInvalid
        {
            internal AuthorizationResultHandle(IntPtr handle) : base(ownsHandle: true)
            {
                SetHandle(handle);
            }

            protected override bool ReleaseHandle()
            {
                FreeResult(handle);
                return true;
            }
        }
    }
}
