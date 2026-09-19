<#import "footer.ftl" as loginFooter>
<#import "scene.ftl" as scene>
<#macro registrationLayout bodyClass="" displayInfo=false displayMessage=true displayRequiredFields=false>
<!DOCTYPE html>
<html class="${properties.kcHtmlClass!}" lang="${lang}"<#if realm.internationalizationEnabled> dir="${(locale.rtl)?then('rtl','ltr')}"</#if>>

<head>
    <meta charset="utf-8">
    <meta http-equiv="Content-Type" content="text/html; charset=UTF-8" />
    <#if properties.meta?has_content>
        <#list properties.meta?split(' ') as meta>
            <meta name="${meta?split('==')[0]}" content="${meta?split('==')[1]}"/>
        </#list>
    </#if>
    <meta name="color-scheme" content="light">
    <title>${title!}</title>
    <link rel="icon" href="${url.resourcesPath}/img/favicon.svg" type="image/svg+xml" />
    <#if properties.styles?has_content>
        <#list properties.styles?split(' ') as style>
            <link href="${url.resourcesPath}/${style}" rel="stylesheet" />
        </#list>
    </#if>
    <#if properties.scripts?has_content>
        <#list properties.scripts?split(' ') as script>
            <script src="${url.resourcesPath}/${script}" type="text/javascript"></script>
        </#list>
    </#if>
    <script type="importmap">
        {
            "imports": {
                "rfc4648": "${url.resourcesCommonPath}/vendor/rfc4648/rfc4648.js"
            }
        }
    </script>
    <#if scripts??>
        <#list scripts as script>
            <script src="${script}" type="text/javascript"></script>
        </#list>
    </#if>
    <script type="module">
        <#outputformat "JavaScript">
        import { startSessionPolling } from ${(url.resourcesPath + "/js/authChecker.js")?c};

        startSessionPolling(
            ${url.ssoLoginInOtherTabsUrl?c}
        );
        </#outputformat>
    </script>
    <script type="module">
        document.addEventListener("click", (event) => {
            const link = event.target.closest("a[data-once-link]");
            if (!link) {
                return;
            }
            if (link.getAttribute("aria-disabled") === "true") {
                event.preventDefault();
                return;
            }
            link.setAttribute("role", "link");
            link.setAttribute("aria-disabled", "true");
        });
    </script>
    <#if authenticationSession??>
        <script type="module">
            <#outputformat "JavaScript">
            import { checkAuthSession } from ${(url.resourcesPath + "/js/authChecker.js")?c};

            checkAuthSession(
                ${authenticationSession.authSessionIdHash?c}
            );
            </#outputformat>
        </script>
    </#if>
    <script>
        // The device approval page cannot see the code a computer showed. The link that opened this tab
        // carries it: Keycloak drops the query when it redirects, but the fragment the Bridge adds survives.
        // Keep it for this tab so the approval page can ask the user to compare the two.
        (function () {
            try {
                var code = new URLSearchParams(window.location.search).get("user_code")
                    || new URLSearchParams(window.location.hash.slice(1)).get("user_code");
                if (code) {
                    sessionStorage.setItem("agent-room-device-code", code);
                }
            } catch (ignored) {
            }
        })();
    </script>
</head>

<body class="${properties.kcBodyClass!} ${bodyClass}" data-page-id="login-${pageId}">
<div class="ar-page">
    <header class="ar-topbar">
        <#if (client.baseUrl)?has_content>
            <a class="ar-brand" href="${client.baseUrl}"><@scene.logo/><span>Agent Room</span></a>
        <#else>
            <span class="ar-brand"><@scene.logo/><span>Agent Room</span></span>
        </#if>
        <#if realm.internationalizationEnabled && locale.supported?size gt 1>
            <nav class="ar-locale" id="kc-locale" aria-label="${msg("languages")}">
                <#-- Each language is named in itself, the way people look for it in a switch. -->
                <#list locale.supported as l>
                    <a class="ar-locale__item" href="${l.url}" lang="${l.languageTag}"<#if l.languageTag == locale.currentLanguageTag> aria-current="true"</#if>><#if l.languageTag == "zh-Hans">中文<#elseif l.languageTag == "en">English<#else>${l.label}</#if></a>
                </#list>
            </nav>
        </#if>
    </header>

    <main class="ar-main">
        <section class="ar-card-column" aria-labelledby="kc-page-title">
            <div class="ar-card">
                <#assign pageHeader><#nested "header"></#assign>
                <#if auth?has_content && auth.showUsername() && !auth.showResetCredentials()>
                    <#nested "show-username">
                    <h1 class="ar-title" id="kc-page-title">${pageHeader}</h1>
                    <div class="ar-account" id="kc-username">
                        <span class="ar-account__name" id="kc-attempted-username">${auth.attemptedUsername}</span>
                        <a class="ar-link" id="reset-login" href="${url.loginRestartFlowUrl}">${msg("arSwitchAccount")}</a>
                    </div>
                <#else>
                    <h1 class="ar-title" id="kc-page-title">${pageHeader}</h1>
                </#if>

                <#-- App-initiated actions should not see warnings about the action they are completing. -->
                <#if displayMessage && message?has_content && (message.type != 'warning' || !isAppInitiatedAction??)>
                    <div class="ar-alert ar-alert--${message.type}" role="<#if message.type = 'error'>alert<#else>status</#if>">
                        <span class="ar-alert__icon" aria-hidden="true"></span>
                        <span class="ar-alert__text">${kcSanitize(message.summary)?no_esc}</span>
                    </div>
                </#if>

                <#nested "form">

                <#if auth?has_content && auth.showTryAnotherWayLink()>
                    <form id="kc-select-try-another-way-form" action="${url.loginAction}" method="post">
                        <input type="hidden" name="tryAnotherWay" value="on"/>
                        <a class="ar-link" href="#" id="try-another-way"
                           onclick="document.forms['kc-select-try-another-way-form'].requestSubmit();return false;">${msg("doTryAnotherWay")}</a>
                    </form>
                </#if>

                <#if switchOrganizationEnabled?? && switchOrganizationEnabled>
                    <form id="kc-switch-organization-form" action="${url.loginAction}" method="post">
                        <input type="hidden" name="switchOrganization" value="true"/>
                        <a class="ar-link" href="#" id="switch-organization"
                           onclick="document.forms['kc-switch-organization-form'].requestSubmit();return false;">${msg("doSwitchOrganization")}</a>
                    </form>
                </#if>

                <#nested "socialProviders">

                <#if displayInfo>
                    <div id="kc-info" class="ar-signup">
                        <#nested "info">
                    </div>
                </#if>
            </div>
            <p class="ar-tagline">${msg("arTagline")}</p>
            <@loginFooter.content/>
        </section>

        <@scene.room page=pageId/>
    </main>
</div>
</body>
</html>
</#macro>
