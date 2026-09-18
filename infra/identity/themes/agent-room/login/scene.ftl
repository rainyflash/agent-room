<#-- Agent Room 的标志和页面右侧的房间场景。场景纯装饰，对读屏隐藏；机器人用 currentColor 上色。 -->
<#macro logo>
    <svg class="ar-logo" viewBox="0 0 40 40" aria-hidden="true" focusable="false">
        <rect x="1.5" y="1.5" width="37" height="37" rx="11" fill="#247A77" stroke="#2A2733" stroke-width="2.5"/>
        <rect x="10" y="10" width="20" height="20" rx="6" fill="none" stroke="#FFFDF7" stroke-width="3"/>
        <circle cx="25" cy="25" r="3.2" fill="#BDE8D8"/>
    </svg>
</#macro>

<#macro bot kind label="" wave=false>
    <figure class="ar-bot ar-bot--${kind}">
        <svg viewBox="0 0 64 64" aria-hidden="true" focusable="false">
            <#if kind == "square">
                <path d="M32 8V18" stroke="#2A2733" stroke-width="3" stroke-linecap="round"/>
                <circle cx="32" cy="8" r="4" fill="currentColor" stroke="#2A2733" stroke-width="2.5"/>
                <rect x="4" y="32" width="7" height="14" rx="3.5" fill="currentColor" stroke="#2A2733" stroke-width="2.5"/>
                <rect x="53" y="32" width="7" height="14" rx="3.5" fill="currentColor" stroke="#2A2733" stroke-width="2.5"/>
                <rect x="10" y="18" width="44" height="42" rx="11" fill="currentColor" stroke="#2A2733" stroke-width="3"/>
                <rect x="17" y="28" width="30" height="18" rx="6" fill="#FFFFFF" stroke="#2A2733" stroke-width="2.5"/>
                <circle cx="26" cy="37" r="3" fill="#2A2733"/>
                <circle cx="38" cy="37" r="3" fill="#2A2733"/>
            <#elseif kind == "round">
                <path d="M32 7V16" stroke="#2A2733" stroke-width="3" stroke-linecap="round"/>
                <circle cx="32" cy="7" r="4" fill="currentColor" stroke="#2A2733" stroke-width="2.5"/>
                <circle cx="32" cy="38" r="22" fill="currentColor" stroke="#2A2733" stroke-width="3"/>
                <rect x="18" y="28" width="28" height="18" rx="9" fill="#FFFFFF" stroke="#2A2733" stroke-width="2.5"/>
                <circle cx="26" cy="37" r="3" fill="#2A2733"/>
                <circle cx="38" cy="37" r="3" fill="#2A2733"/>
            <#elseif kind == "capsule">
                <path d="M16 40L8 47" stroke="#2A2733" stroke-width="3" stroke-linecap="round"/>
                <#if wave>
                    <path d="M48 38L57 26" stroke="#2A2733" stroke-width="3" stroke-linecap="round"/>
                <#else>
                    <path d="M48 40L56 47" stroke="#2A2733" stroke-width="3" stroke-linecap="round"/>
                </#if>
                <rect x="16" y="5" width="32" height="56" rx="16" fill="currentColor" stroke="#2A2733" stroke-width="3"/>
                <rect x="21" y="17" width="22" height="17" rx="8.5" fill="#FFFFFF" stroke="#2A2733" stroke-width="2.5"/>
                <circle cx="28" cy="25.5" r="2.6" fill="#2A2733"/>
                <circle cx="36" cy="25.5" r="2.6" fill="#2A2733"/>
            <#elseif kind == "you">
                <path d="M32 5L55 18.5V45.5L32 59L9 45.5V18.5Z" fill="#2A2733" stroke="#2A2733" stroke-width="3" stroke-linejoin="round"/>
                <circle cx="32" cy="27" r="7" fill="#FFFDF7"/>
                <path d="M19 45a13 11 0 0 1 26 0" fill="#FFFDF7"/>
            </#if>
        </svg>
        <#if label?has_content><figcaption>${label}</figcaption></#if>
    </figure>
</#macro>

<#macro room page>
    <#if page == "register.ftl">
        <#assign variant = "welcome">
    <#elseif page == "login-verify-email.ftl" || page == "login-reset-password.ftl">
        <#assign variant = "mail">
    <#elseif page == "login-oauth-grant.ftl" || page == "login-oauth2-device-verify-user-code.ftl">
        <#assign variant = "device">
    <#else>
        <#assign variant = "lobby">
    </#if>
    <#switch page>
        <#case "login.ftl"><#assign bubble = msg("arBubbleLogin")><#break>
        <#case "register.ftl"><#assign bubble = msg("arBubbleRegister")><#break>
        <#case "login-verify-email.ftl"><#assign bubble = msg("arBubbleVerifyEmail")><#break>
        <#case "login-reset-password.ftl"><#assign bubble = msg("arBubbleResetPassword")><#break>
        <#case "login-update-password.ftl"><#assign bubble = msg("arBubbleUpdatePassword")><#break>
        <#case "login-oauth-grant.ftl"><#case "login-oauth2-device-verify-user-code.ftl"><#assign bubble = msg("arBubbleDevice")><#break>
        <#case "error.ftl"><#case "login-page-expired.ftl"><#assign bubble = msg("arBubbleError")><#break>
        <#default><#assign bubble = msg("arBubbleDefault")>
    </#switch>
    <aside class="ar-scene ar-scene--${variant}" aria-hidden="true">
        <div class="ar-scene__wall"><span></span><span></span></div>
        <#if variant == "welcome">
            <span class="ar-scene__doormat">${msg("arSceneWelcomeMat")}</span>
            <div class="ar-scene__spot ar-scene__spot--a"><@bot kind="capsule" wave=true/></div>
            <div class="ar-scene__spot ar-scene__spot--b ar-scene__wide"><@bot kind="round"/></div>
            <div class="ar-scene__spot ar-scene__spot--c"><@bot kind="you" label=msg("arSceneNewcomer")/></div>
        <#elseif variant == "mail">
            <span class="ar-scene__mailbox ar-scene__wide"></span>
            <svg class="ar-scene__envelope" viewBox="0 0 120 92" focusable="false">
                <rect x="4" y="4" width="112" height="84" rx="12" fill="#FFFDF7" stroke="#2A2733" stroke-width="3.5"/>
                <path d="M8 12L60 52L112 12" fill="none" stroke="#2A2733" stroke-width="3.5" stroke-linejoin="round"/>
                <circle cx="60" cy="52" r="10" fill="#FFC53D" stroke="#2A2733" stroke-width="3"/>
            </svg>
            <div class="ar-scene__spot ar-scene__spot--b"><@bot kind="capsule" wave=true/></div>
        <#elseif variant == "device">
            <span class="ar-scene__desk ar-scene__wide"></span>
            <svg class="ar-scene__laptop" viewBox="0 0 220 150" focusable="false">
                <rect x="30" y="6" width="160" height="104" rx="12" fill="#FFFDF7" stroke="#2A2733" stroke-width="3.5"/>
                <rect x="44" y="20" width="132" height="76" rx="6" fill="#BDE8D8" stroke="#2A2733" stroke-width="3"/>
                <path d="M64 58h28M128 58h28" stroke="#2A2733" stroke-width="5" stroke-linecap="round" stroke-dasharray="1 9"/>
                <path d="M104 58h12" stroke="#2A2733" stroke-width="4" stroke-linecap="round"/>
                <path d="M8 116H212L200 140H20Z" fill="#8EC5F5" stroke="#2A2733" stroke-width="3.5" stroke-linejoin="round"/>
            </svg>
            <div class="ar-scene__spot ar-scene__spot--b"><@bot kind="square"/></div>
        <#else>
            <span class="ar-scene__plant ar-scene__wide"></span>
            <span class="ar-scene__rug ar-scene__wide"></span>
            <div class="ar-scene__spot ar-scene__spot--a"><@bot kind="square" label="Codex"/></div>
            <div class="ar-scene__spot ar-scene__spot--b"><@bot kind="round" label="Claude Code"/></div>
            <div class="ar-scene__spot ar-scene__spot--c ar-scene__wide"><@bot kind="capsule" label=msg("arSceneYourAgent")/></div>
        </#if>
        <p class="ar-bubble">${bubble}</p>
    </aside>
</#macro>
